//! Edit → Edit Colors: Adjust Color Balance modes and the live preview.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::Node;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::tooling::ViewInfo;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A selected rectangle filled with `fill`.
fn rect(s: &mut Session, fill: Value) -> NodeId {
    let id = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": fill})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    id
}

fn fill_of(s: &Session, id: NodeId) -> Color {
    s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().color().unwrap()
}

#[test]
fn adjust_balance_modes() {
    let mut s = session();
    let a = rect(&mut s, json!({"c": 20, "m": 40, "y": 0, "k": 10}));
    // CMYK channels on a CMYK colour stay CMYK.
    s.execute("edit.colors.adjustBalance", &json!({"mode": "cmyk", "c": 10, "k": -10})).unwrap();
    let Color::Cmyk { c, m, k, .. } = fill_of(&s, a) else { panic!("kept CMYK") };
    assert!((c - 0.3).abs() < 1e-5 && (m - 0.4).abs() < 1e-5 && k.abs() < 1e-5, "{c} {m} {k}");
    // RGB channels keep the colour's model unless Convert is on.
    s.execute("edit.colors.adjustBalance", &json!({"mode": "rgb", "r": 10})).unwrap();
    assert!(matches!(fill_of(&s, a), Color::Cmyk { .. }));
    s.execute("edit.colors.adjustBalance", &json!({"mode": "rgb", "convert": true})).unwrap();
    assert!(matches!(fill_of(&s, a), Color::Rgb { .. }), "converted to RGB");
    // Gray with Convert makes a grey of the colour darkened by the shift.
    let b = rect(&mut s, json!("#808080"));
    s.execute("edit.colors.adjustBalance", &json!({"mode": "gray", "gray": 20, "convert": true})).unwrap();
    let Color::Gray { k } = fill_of(&s, b) else { panic!("converted to grey") };
    assert!((k - 0.698).abs() < 0.01, "{k}");
    // Mode follows the channels given when it is left out.
    s.execute("edit.colors.adjustBalance", &json!({"gray": -20})).unwrap();
    assert!(matches!(fill_of(&s, b), Color::Gray { .. }));
    assert!(s.execute("edit.colors.adjustBalance", &json!({"mode": "lab"})).is_err());
}

#[test]
fn balance_previews_and_commits_one_step() {
    let mut s = session();
    let a = rect(&mut s, json!("#808080"));
    let undo = s.doc().unwrap().history.undo.len();
    s.begin_interaction("Adjust Colors").unwrap();
    for r in [10, 30, 20] {
        s.preview("edit.colors.adjustBalance", &json!({"mode": "rgb", "r": r})).unwrap();
    }
    assert_eq!(fill_of(&s, a).to_hex(), "#b38080", "each preview starts from the original");
    s.commit_interaction().unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(fill_of(&s, a).to_hex(), "#808080");
}

/// Saturate needs its intensity: a call without one, or with one that is not a number, is an error
/// that changes nothing and records no undo step. A number outside −100..100 counts as its nearest end.
#[test]
fn saturate_needs_a_number_intensity() {
    let mut s = session();
    let a = rect(&mut s, json!("#cc6666"));
    let undo = s.doc().unwrap().history.undo.len();
    for p in [json!({}), json!({"intensity": null})] {
        let e = s.execute("edit.colors.saturate", &p).unwrap_err().to_string();
        assert!(e.contains("missing number `intensity`"), "{p}: {e}");
    }
    let e = s.execute("edit.colors.saturate", &json!({"intensity": "40"})).unwrap_err().to_string();
    assert!(e.contains("`intensity` must be a number, not \"40\""), "{e}");
    assert_eq!(fill_of(&s, a).to_hex(), "#cc6666");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "no undo step");
    s.execute("edit.colors.saturate", &json!({"intensity": 100})).unwrap();
    let full = fill_of(&s, a).to_hex();
    assert_ne!(full, "#cc6666");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.colors.saturate", &json!({"intensity": 250})).unwrap();
    assert_eq!(fill_of(&s, a).to_hex(), full, "250 counts as 100");
}

/// Saturate at 0 leaves every color as it was: a swatch-linked fill keeps its exact color and its link.
#[test]
fn saturate_keeps_a_color_it_does_not_change() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#3a7bd5", "global": true})).unwrap();
    let a = rect(&mut s, json!("#3a7bd5"));
    s.execute("paint.setFill", &json!({"ids": [a.0], "swatch": "Sky"})).unwrap();
    let b = rect(&mut s, json!({"c": 20, "m": 40, "y": 0, "k": 10}));
    let paint = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().clone();
    let (sky, cmyk) = (paint(&s, a), paint(&s, b));
    let v = s.execute("edit.colors.saturate", &json!({"ids": [a.0, b.0], "intensity": 0})).unwrap();
    assert_eq!(v["changed"], 0);
    assert_eq!(paint(&s, a), sky);
    assert_eq!(paint(&s, b), cmyk);
}

/// Adjust Color Balance refuses a channel that is not a number, which it would otherwise read as 0.
#[test]
fn adjust_balance_refuses_a_channel_that_is_not_a_number() {
    let mut s = session();
    let a = rect(&mut s, json!("#808080"));
    let undo = s.doc().unwrap().history.undo.len();
    for p in [json!({"mode": "rgb", "r": "40"}), json!({"c": "10"}), json!({"gray": true}), json!({"tint": "5"})] {
        let e = s.execute("edit.colors.adjustBalance", &p).unwrap_err().to_string();
        assert!(e.contains("must be a number"), "{p}: {e}");
    }
    assert_eq!(fill_of(&s, a).to_hex(), "#808080");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "no undo step");
    // A null is read as a channel left out.
    s.execute("edit.colors.adjustBalance", &json!({"mode": "rgb", "r": 20, "g": null})).unwrap();
    assert_eq!(fill_of(&s, a).to_hex(), "#b38080");
}

/// The Edit Colors commands refuse a scope switch, `convert` or `mode` of the wrong type, which they
/// would otherwise replace with the default: `{"fill": "false"}` recolored the fills.
#[test]
fn edit_colors_refuse_a_param_of_the_wrong_type() {
    let mut s = session();
    let a = rect(&mut s, json!("#ff0000"));
    let undo = s.doc().unwrap().history.undo.len();
    let cases = [
        ("edit.colors.invert", json!({"fill": "false"}), "`fill` must be true or false"),
        ("edit.colors.toCMYK", json!({"stroke": 0}), "`stroke` must be true or false"),
        ("edit.colors.toGrayscale", json!({"fill": "false"}), "`fill` must be true or false"),
        ("edit.colors.toRGB", json!({"includeImages": "no"}), "`includeImages` must be true or false"),
        ("edit.colors.saturate", json!({"intensity": 50, "includePatterns": 1}), "`includePatterns` must be true or false"),
        ("edit.colors.adjustBalance", json!({"r": 20, "fill": "false"}), "`fill` must be true or false"),
        ("edit.colors.adjustBalance", json!({"tint": 20, "convert": "true"}), "`convert` must be true or false"),
        ("edit.colors.adjustBalance", json!({"tint": 20, "fill": "false"}), "`fill` must be true or false"),
        ("edit.colors.adjustBalance", json!({"mode": 1, "r": 20}), "`mode` must be a string"),
    ];
    for (id, p, want) in cases {
        let e = s.execute(id, &p).unwrap_err().to_string();
        assert!(e.contains(want), "{id} {p}: {e}");
    }
    assert_eq!(fill_of(&s, a).to_hex(), "#ff0000");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "no undo step");
    // A null is read as a switch left out.
    s.execute("edit.colors.invert", &json!({"fill": null, "stroke": null})).unwrap();
    assert_eq!(fill_of(&s, a).to_hex(), "#00ffff");
}

// ---------- meshes, images and patterns ----------

fn mesh_colors(s: &Session, id: NodeId) -> Vec<Color> {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Mesh(m) => m.points.iter().map(|p| p.color).collect(),
        _ => panic!("not a mesh"),
    }
}

/// A selected 1×1 gradient mesh: red, with one green corner.
fn mesh(s: &mut Session) -> NodeId {
    let a = rect(s, json!("#ff0000"));
    s.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
    s.execute("object.mesh.setPointColor", &json!({"id": a.0, "index": 0, "color": "#00ff00"})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    a
}

#[test]
fn edit_colors_reach_mesh_points() {
    let mut s = session();
    let m = mesh(&mut s);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("edit.colors.toGrayscale", &json!({})).unwrap();
    let pts = mesh_colors(&s, m);
    assert!(pts.iter().all(|c| matches!(c, Color::Gray { .. })), "a grey mesh: {pts:?}");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    s.execute("edit.colors.invert", &json!({"fill": false})).unwrap();
    assert_eq!(mesh_colors(&s, m), pts, "a mesh's points are fill colours");
}

/// Clicking a gradient mesh's point with the Mesh tool or Direct Selection picks it: a solid fill
/// colour then recolours that point alone (#1050).
#[test]
fn a_clicked_mesh_point_takes_the_fill_colour() {
    let mut s = session();
    let m = mesh(&mut s);
    let before = mesh_colors(&s, m);
    let v = ViewInfo::default();
    let click = |s: &mut Session, i: usize| {
        let NodeKind::Mesh(g) = &s.doc().unwrap().doc.node(m).unwrap().kind else { panic!("not a mesh") };
        let q = g.points[i].p;
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, q.x, q.y), v).unwrap();
        }
    };
    // The Mesh tool selects the mesh whose point it clicks.
    s.execute("select.none", &json!({})).unwrap();
    s.select_tool("mesh", v).unwrap();
    click(&mut s, 1);
    assert_eq!(s.doc().unwrap().selection.objects, [m]);
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    let after = mesh_colors(&s, m);
    assert_eq!(after[1].to_hex(), "#0000ff", "the clicked point takes the colour");
    assert_eq!((after[0], after[2], after[3]), (before[0], before[2], before[3]), "the other points keep theirs");
    s.select_tool("directSelection", v).unwrap();
    click(&mut s, 2);
    s.execute("paint.setFill", &json!({"color": "#ffff00"})).unwrap();
    let after = mesh_colors(&s, m);
    assert_eq!((after[1].to_hex(), after[2].to_hex()), ("#0000ff".to_string(), "#ffff00".to_string()));
    assert_eq!((after[0], after[3]), (before[0], before[3]));
}

fn mesh_opacities(s: &Session, id: NodeId) -> Vec<f32> {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Mesh(m) => m.points.iter().map(|p| p.opacity).collect(),
        _ => panic!("not a mesh"),
    }
}

/// The Transparency panel's opacity on a clicked mesh point fades that point alone, not the whole
/// object (#4206).
#[test]
fn a_clicked_mesh_point_takes_the_opacity() {
    let mut s = session();
    let m = rect(&mut s, json!("#ff0000"));
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    s.execute("select.set", &json!({"ids": [m.0]})).unwrap();
    let v = ViewInfo::default();
    let click = |s: &mut Session, i: usize| {
        let NodeKind::Mesh(g) = &s.doc().unwrap().doc.node(m).unwrap().kind else { panic!("not a mesh") };
        let q = g.points[i].p;
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, q.x, q.y), v).unwrap();
        }
    };
    let object_opacity = |s: &Session| s.doc().unwrap().doc.node(m).unwrap().opacity;
    for (tool, point) in [("mesh", 4), ("directSelection", 4)] {
        s.select_tool(tool, v).unwrap();
        click(&mut s, point);
        let undo = s.doc().unwrap().history.undo.len();
        s.execute("transparency.set", &json!({"opacity": 30})).unwrap();
        let o = mesh_opacities(&s, m);
        assert!((o[4] - 0.3).abs() < 1e-6, "{tool}: the clicked point fades: {o:?}");
        assert!(o.iter().enumerate().all(|(i, a)| i == 4 || *a == 1.0), "{tool}: the other points stay opaque: {o:?}");
        assert_eq!(object_opacity(&s), 1.0, "{tool}: the object stays opaque");
        assert_eq!(s.transparency_info().opacity, Some(30.0), "{tool}: the panel shows the point's opacity");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(mesh_opacities(&s, m).iter().all(|a| *a == 1.0));
    }
    // With no point focused the object takes the opacity.
    s.select_tool("selection", v).unwrap();
    s.execute("select.set", &json!({"ids": [m.0]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 30})).unwrap();
    assert!((object_opacity(&s) - 0.3).abs() < 1e-6);
    assert!(mesh_opacities(&s, m).iter().all(|a| *a == 1.0));
}

/// Mesh points picked together (Shift-clicks with the Mesh tool, a Direct Selection marquee) all
/// take a solid fill colour and the Transparency panel's opacity, and only they do; the panel
/// shows their opacity, blank when it differs; a drag of one moves them all as one undo step
/// (#4207, #4208, #4170).
#[test]
fn picked_mesh_points_take_the_fill_opacity_and_drag_together() {
    let mut s = session();
    let m = rect(&mut s, json!("#ff0000"));
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    s.execute("select.set", &json!({"ids": [m.0]})).unwrap();
    let v = ViewInfo::default();
    let point = |s: &Session, i: usize| {
        let NodeKind::Mesh(g) = &s.doc().unwrap().doc.node(m).unwrap().kind else { panic!("not a mesh") };
        g.points[i].p
    };
    let pointer = |s: &mut Session, kind: PointerKind, q: vectorcraft_geom::Point, shift: bool| {
        let mut ev = PointerEvent::new(kind, q.x, q.y);
        ev.mods.shift = shift;
        s.pointer(&ev, v).unwrap();
    };
    let click = |s: &mut Session, i: usize, shift: bool| {
        let q = point(s, i);
        pointer(s, PointerKind::Down, q, shift);
        pointer(s, PointerKind::Up, q, shift);
    };
    let check = |s: &Session, picked: &[usize], hex: &str, opacity: f32, what: &str| {
        let (c, o) = (mesh_colors(s, m), mesh_opacities(s, m));
        for i in 0..c.len() {
            let on = picked.contains(&i);
            assert_eq!(c[i].to_hex() == hex, on, "{what}: point {i} colour {c:?}");
            assert_eq!((o[i] - opacity).abs() < 1e-6, on, "{what}: point {i} opacity {o:?}");
        }
        assert_eq!(s.doc().unwrap().doc.node(m).unwrap().opacity, 1.0, "{what}: the object stays opaque");
    };
    // The Mesh tool: a click, then Shift-clicks add points (and a second one takes one away).
    s.select_tool("mesh", v).unwrap();
    click(&mut s, 1, false);
    for i in [4, 8, 8] {
        click(&mut s, i, true);
    }
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("transparency.set", &json!({"opacity": 40})).unwrap();
    check(&s, &[1, 4], "#0000ff", 0.4, "mesh tool");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step for both points");
    assert_eq!(s.transparency_info().opacity, Some(40.0), "the panel shows the points' opacity");
    s.execute("object.mesh.setPointColor", &json!({"id": m.0, "index": 4, "opacity": 0.9})).unwrap();
    assert_eq!(s.transparency_info().opacity, None, "mixed: blank");
    // Direct Selection: a marquee over the top-left patch picks its four points.
    s.select_tool("directSelection", v).unwrap();
    let corner = point(&s, 0);
    pointer(&mut s, PointerKind::Down, corner - vectorcraft_geom::Vec2::new(8.0, 8.0), false);
    let far = point(&s, 4) + vectorcraft_geom::Vec2::new(5.0, 5.0);
    pointer(&mut s, PointerKind::Drag, far, false);
    pointer(&mut s, PointerKind::Up, far, false);
    assert_eq!(s.doc().unwrap().selection.objects, [m], "the marquee selects the mesh");
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 20})).unwrap();
    check(&s, &[0, 1, 3, 4], "#00ff00", 0.2, "marquee");
    // A drag of one picked point moves all four by the same offset, in one step.
    let before: Vec<_> = (0..9).map(|i| point(&s, i)).collect();
    let undo = s.doc().unwrap().history.undo.len();
    let d = vectorcraft_geom::Vec2::new(4.0, 6.0);
    pointer(&mut s, PointerKind::Down, before[4], false);
    pointer(&mut s, PointerKind::Drag, before[4] + d, false);
    pointer(&mut s, PointerKind::Up, before[4] + d, false);
    for (i, was) in before.iter().enumerate() {
        let want = if [0, 1, 3, 4].contains(&i) { *was + d } else { *was };
        assert!(point(&s, i).distance(want) < 1e-9, "point {i}: {:?} ≠ {want:?}", point(&s, i));
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((0..9).map(|i| point(&s, i)).collect::<Vec<_>>(), before);
}

#[test]
fn blend_moves_a_mesh_to_the_blended_colour() {
    let mut s = session();
    let left = rect(&mut s, json!("#000000"));
    let m = mesh(&mut s);
    s.execute("object.transform", &json!({"ids": [m.0], "matrix": [1, 0, 0, 1, 100, 0]})).unwrap();
    let right = rect(&mut s, json!("#ffffff"));
    s.execute("object.transform", &json!({"ids": [right.0], "matrix": [1, 0, 0, 1, 200, 0]})).unwrap();
    s.execute("select.set", &json!({"ids": [left.0, m.0, right.0]})).unwrap();
    let before = mesh_colors(&s, m);
    s.execute("edit.colors.blendHorizontally", &json!({})).unwrap();
    let after = mesh_colors(&s, m);
    let mean = |cs: &[Color]| cs.iter().map(|c| c.to_rgb().iter().sum::<f32>() / 3.0).sum::<f32>() / cs.len() as f32;
    assert!((mean(&after) - 0.5).abs() < 0.03, "{}", mean(&after));
    assert_ne!(after[0], after[1], "the shading is kept");
    assert_ne!(before, after);
}

/// A 2×2 RGBA image as a PNG.
fn png(px: [[u8; 4]; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_raw(2, 2, px.concat()).unwrap();
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn pixels(s: &Session, id: NodeId) -> Vec<[u8; 4]> {
    let d = &s.doc().unwrap().doc;
    let NodeKind::Image(im) = &d.node(id).unwrap().kind else { panic!("not an image") };
    image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect()
}

/// A selected embedded 2×2 image.
fn image_node(s: &mut Session) -> NodeId {
    image_of(s, [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 128]])
}

/// A selected embedded 2×2 image of the pixels `px`.
fn image_of(s: &mut Session, px: [[u8; 4]; 4]) -> NodeId {
    let bytes = png(px);
    s.edit("Place", |d, sel| {
        d.images.insert("pic".into(), vectorcraft_doc::ImageBlob::png(bytes));
        let id = d.alloc_id();
        let im = vectorcraft_doc::ImageObject {
            key: "pic".into(),
            width: 2,
            height: 2,
            xf: Default::default(),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

#[test]
fn invert_recolours_an_embedded_image() {
    let mut s = session();
    let id = image_node(&mut s);
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("edit.colors.invert", &json!({})).unwrap();
    assert_eq!(r["changed"], 1);
    assert_eq!(pixels(&s, id), [[0, 255, 255, 255], [255, 0, 255, 255], [255, 255, 0, 255], [0, 0, 0, 128]], "inverted, alpha kept");
    let d = &s.doc().unwrap().doc;
    assert!(d.images.contains_key("pic"), "the original image stays for undo and other objects");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(pixels(&s, id)[0], [255, 0, 0, 255]);
    // Without images, nothing changes.
    let r = s.execute("edit.colors.invert", &json!({"includeImages": false})).unwrap();
    assert_eq!(r["changed"], 0);
    // Recolor maps exact pixel colours.
    s.execute("recolor.apply", &json!({"map": {"#ff0000": "#000000"}})).unwrap();
    assert_eq!(pixels(&s, id)[..2], [[0, 0, 0, 255], [0, 255, 0, 255]]);
}

/// The key of image object `id`'s blob.
fn image_key(s: &Session, id: NodeId) -> String {
    let NodeKind::Image(im) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("not an image") };
    im.key.clone()
}

/// Convert to Grayscale stores an image with one color channel, alpha kept (#5632).
#[test]
fn convert_to_grayscale_stores_an_image_as_grayscale() {
    let mut s = session();
    let id = image_node(&mut s);
    let r = s.execute("edit.colors.toGrayscale", &json!({})).unwrap();
    assert_eq!(r["changed"], 1);
    let key = image_key(&s, id);
    let source = s.doc().unwrap().doc.images[&key].gray_source();
    assert!(source.is_some_and(|g| g.alpha && !g.one_bit), "a grayscale file with alpha: {source:?}");
    let gray = |hex: &str| Color::from_hex(hex).unwrap().in_model(vectorcraft_color::cms::Model::Gray).to_rgba8(1.0)[0];
    let (r_, g_, b_, w) = (gray("#ff0000"), gray("#00ff00"), gray("#0000ff"), gray("#ffffff"));
    assert_eq!(pixels(&s, id), [[r_, r_, r_, 255], [g_, g_, g_, 255], [b_, b_, b_, 255], [w, w, w, 128]]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(pixels(&s, id)[0], [255, 0, 0, 255], "one undo step restores the color image");
    s.execute("edit.redo", &json!({})).unwrap();
    // An image that is already grayscale has nothing to convert.
    let again = s.execute("edit.colors.toGrayscale", &json!({})).unwrap();
    assert_eq!(again["changed"], 0);
    assert_eq!(image_key(&s, id), key);
}

/// An RGB image whose pixels are all neutral still becomes a one-channel image (#5632).
#[test]
fn convert_to_grayscale_stores_a_neutral_rgb_image_as_grayscale() {
    let mut s = session();
    let id = image_of(&mut s, [[255, 255, 255, 255], [0, 0, 0, 255], [255, 255, 255, 255], [0, 0, 0, 255]]);
    let r = s.execute("edit.colors.toGrayscale", &json!({})).unwrap();
    assert_eq!(r["changed"], 1);
    let source = s.doc().unwrap().doc.images[&image_key(&s, id)].gray_source();
    assert!(source.is_some_and(|g| !g.alpha), "an opaque image has no alpha channel: {source:?}");
}

/// Select All leaves a hidden neutral RGB image out of Convert to Grayscale, which would change it.
#[test]
fn a_hidden_neutral_rgb_image_counts_as_not_converted() {
    let mut s = session();
    let hidden = image_of(&mut s, [[255, 255, 255, 255], [0, 0, 0, 255], [255, 255, 255, 255], [0, 0, 0, 255]]);
    s.execute("layer.setProps", &json!({"ids": [hidden.0], "visible": false})).unwrap();
    rect(&mut s, json!("#ff0000"));
    s.execute("select.all", &json!({})).unwrap();
    let r = s.execute("edit.colors.toGrayscale", &json!({})).unwrap();
    assert_eq!(r["notChanged"], json!({"locked": 0, "hidden": 1}), "{r}");
}

/// Adjust Colors converting to gray stores an image with one channel; without Convert it keeps
/// the image's RGB (#5632).
#[test]
fn adjust_colors_converting_to_gray_stores_an_image_as_grayscale() {
    let source = |convert: bool| {
        let mut s = session();
        let id = image_node(&mut s);
        s.execute("edit.colors.adjustBalance", &json!({"mode": "gray", "gray": 10, "convert": convert})).unwrap();
        s.doc().unwrap().doc.images[&image_key(&s, id)].gray_source()
    };
    assert!(source(true).is_some(), "converted to gray");
    assert!(source(false).is_none(), "the colors keep their model");
}

/// Convert to Indexed Color stores an image as a palette PNG in one undo step; an image with few
/// enough colors keeps them, and converting it again changes nothing and adds no undo step.
#[test]
fn convert_to_indexed_stores_an_image_as_a_palette_png() {
    let mut s = session();
    let id = image_node(&mut s);
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("edit.colors.toIndexed", &json!({})).unwrap();
    assert_eq!(r["changed"], 1, "{r}");
    let key = image_key(&s, id);
    assert!(s.doc().unwrap().doc.images[&key].indexed_png(), "an indexed PNG");
    assert_eq!(pixels(&s, id), [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 255]], "half opaque becomes opaque");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let again = s.execute("edit.colors.toIndexed", &json!({})).unwrap();
    assert_eq!(again["changed"], 0, "{again}");
    assert_eq!(image_key(&s, id), key);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "no undo step when nothing changes");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(pixels(&s, id)[3], [255, 255, 255, 128], "one undo step restores the image");
}

/// Pixels under half opacity become transparent; an indexed image converted to a palette it
/// already shows is left as it is, and one converted to a smaller palette changes.
#[test]
fn convert_to_indexed_keeps_transparency_and_reduces_an_indexed_image() {
    let mut s = session();
    let id = image_of(&mut s, [[200, 30, 90, 255], [20, 40, 220, 255], [0, 0, 0, 100], [250, 250, 250, 255]]);
    s.execute("edit.colors.toIndexed", &json!({"dither": "none"})).unwrap();
    assert_eq!(pixels(&s, id)[2][3], 0, "under half opacity: transparent");
    let r = s.execute("edit.colors.toIndexed", &json!({"reduction": "blackWhite", "dither": "none"})).unwrap();
    assert_eq!(r["changed"], 1, "{r}");
    let px = pixels(&s, id);
    assert_eq!([px[0], px[1], px[3]], [[0, 0, 0, 255], [0, 0, 0, 255], [255, 255, 255, 255]]);
    assert_eq!(px[2][3], 0);
}

/// The uniform reduction takes evenly spaced levels of each channel, and the document reduction
/// the colors of the document's swatches.
#[test]
fn convert_to_indexed_uses_a_uniform_or_the_document_palette() {
    let mut s = session();
    let id = image_of(&mut s, [[200, 30, 90, 255], [20, 40, 220, 255], [120, 130, 140, 255], [250, 250, 250, 255]]);
    s.execute("edit.colors.toIndexed", &json!({"reduction": "uniform", "colors": 9, "dither": "none"})).unwrap();
    assert_eq!(pixels(&s, id), [[255, 0, 0, 255], [0, 0, 255, 255], [0, 255, 255, 255], [255, 255, 255, 255]]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.colors.toIndexed", &json!({"reduction": "document"})).unwrap();
    let swatches: Vec<[u8; 3]> = crate::cmd::swatchlib::library_colors(&s, "document")
        .unwrap()
        .iter()
        .map(|c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            [r, g, b]
        })
        .collect();
    for p in pixels(&s, id) {
        assert!(swatches.contains(&[p[0], p[1], p[2]]), "{p:?} is a swatch color");
    }
}

/// The start of a PNG of `w` × `h` RGBA pixels: its header and a first image data chunk, without
/// the rest of the pixels.
fn png_header(w: u32, h: u32) -> Vec<u8> {
    let crc = |data: &[u8]| {
        let mut c = !0u32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    };
    let chunk = |ty: &[u8; 4], data: &[u8]| {
        let body = [ty.as_slice(), data].concat();
        [(data.len() as u32).to_be_bytes().as_slice(), &body, &crc(&body).to_be_bytes()].concat()
    };
    let ihdr = [w.to_be_bytes().as_slice(), &h.to_be_bytes(), &[8, 6, 0, 0, 0]].concat();
    [b"\x89PNG\r\n\x1a\n".as_slice(), &chunk(b"IHDR", &ihdr), &chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0x00])].concat()
}

/// An image larger than Convert to Indexed Color converts is refused by its size, read from the
/// file's header, before any pixel is decoded; nothing changes.
#[test]
fn convert_to_indexed_refuses_an_image_too_large_to_convert() {
    let mut s = session();
    let id = image_node(&mut s);
    assert_eq!(vectorcraft_doc::ImageBlob::png(png_header(30_000, 30_000)).pixel_size(), Some((30_000, 30_000)));
    s.edit("Swap", |d, _| {
        d.images.insert("pic".into(), vectorcraft_doc::ImageBlob::png(png_header(30_000, 30_000)));
        Ok(())
    })
    .unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    let e = s.execute("edit.colors.toIndexed", &json!({})).unwrap_err().to_string();
    assert!(e.contains("an image of 30000 × 30000 pixels is too large to convert"), "{e}");
    assert_eq!(image_key(&s, id), "pic");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

/// Convert to Indexed Color refuses parameters it can't use, and leaves the image as it is.
#[test]
fn convert_to_indexed_refuses_bad_params() {
    let mut s = session();
    let id = image_node(&mut s);
    for (p, msg) in [
        (json!({"colors": 1}), "`colors` must be a whole number from 2 to 256"),
        (json!({"colors": 257}), "`colors` must be a whole number from 2 to 256"),
        (json!({"colors": "8"}), "`colors` must be a whole number from 2 to 256"),
        (json!({"colors": 16.5}), "`colors` must be a whole number from 2 to 256"),
        (json!({"reduction": "uniform", "colors": 8}), "`colors` must be at least 9 for the uniform reduction"),
        (json!({"reduction": "document", "colors": 16}), "`colors` does not apply to the document reduction"),
        (json!({"reduction": "blackWhite", "colors": 2}), "`colors` does not apply to the blackWhite reduction"),
        (json!({"reduction": "posterize"}), "unknown reduction `posterize`"),
        (json!({"dither": "random"}), "unknown dither `random`"),
        (json!({"ditherAmount": 101}), "`ditherAmount` must be a number from 0 to 100"),
        (json!({"ditherAmount": "50"}), "`ditherAmount` must be a number from 0 to 100"),
    ] {
        let e = s.execute("edit.colors.toIndexed", &p).unwrap_err().to_string();
        assert!(e.contains(msg), "{p}: {e}");
    }
    assert_eq!(image_key(&s, id), "pic");
}

/// An embedded `w` × `h` image of the pixels `f(x, y)`, placed and selected.
fn placed(s: &mut Session, w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> NodeId {
    let mut png = Vec::new();
    image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y))).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let r = s.execute("file.place", &json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    NodeId(r["ids"][0].as_u64().unwrap())
}

/// `ids` picks the images to convert; a linked image and a colorized image keep their pixels.
#[test]
fn convert_to_indexed_takes_ids_and_leaves_linked_and_colorized_images() {
    let mut s = session();
    let a = placed(&mut s, 2, 1, |x, _| [200, 30 + x as u8, 90, 255]);
    let b = placed(&mut s, 2, 1, |x, _| [20, 40, 200 + x as u8, 255]);
    let (ka, kb) = (image_key(&s, a), image_key(&s, b));
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let r = s.execute("edit.colors.toIndexed", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(r["changed"], 1, "{r}");
    assert_eq!(image_key(&s, a), ka, "not in `ids`");
    assert_ne!(image_key(&s, b), kb);
    s.edit("Link", |d, _| {
        let NodeKind::Image(im) = &mut d.node_mut(a).unwrap().kind else { panic!("not an image") };
        im.link = Some(vectorcraft_doc::LinkInfo::new("art.png"));
        Ok(())
    })
    .unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("edit.colors.toIndexed", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(r["changed"], 0, "a linked image: {r}");
    assert_eq!(image_key(&s, a), ka);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "no undo step");
    let mut png = Vec::new();
    image::GrayImage::from_pixel(4, 4, image::Luma([90])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let r = s.execute("file.place", &json!({"name": "gray.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    let gray = NodeId(r["ids"][0].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [gray.0], "color": "#ff0000"})).unwrap();
    assert!(s.doc().unwrap().doc.node(gray).unwrap().extras().colorize.is_some(), "the fill colorizes the image");
    let kg = image_key(&s, gray);
    let r = s.execute("edit.colors.toIndexed", &json!({"ids": [gray.0]})).unwrap();
    assert_eq!(r["changed"], 0, "a colorized image: {r}");
    assert_eq!(image_key(&s, gray), kg);
}

/// With the selection Select All makes, locked and hidden images are left out and counted in
/// `notChanged`; a left-out image too large to convert is counted without an error.
#[test]
fn convert_to_indexed_counts_left_out_images() {
    let mut s = session();
    let a = placed(&mut s, 2, 1, |x, _| [200, 30 + x as u8, 90, 255]);
    let locked = placed(&mut s, 2, 1, |x, _| [20, 40, 200 + x as u8, 255]);
    let hidden = placed(&mut s, 2, 1, |x, _| [120, 140 + x as u8, 20, 255]);
    s.edit("Huge", |d, _| {
        let NodeKind::Image(im) = &mut d.node_mut(locked).unwrap().kind else { panic!("not an image") };
        im.key = "huge".into();
        d.images.insert("huge".into(), vectorcraft_doc::ImageBlob::png(png_header(30_000, 30_000)));
        Ok(())
    })
    .unwrap();
    s.execute("object.lock", &json!({"ids": [locked.0]})).unwrap();
    s.execute("object.hide", &json!({"ids": [hidden.0]})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let kh = image_key(&s, hidden);
    let r = s.execute("edit.colors.toIndexed", &json!({})).unwrap();
    assert_eq!(r["changed"], 1, "{r}");
    assert_eq!(r["notChanged"], json!({"locked": 1, "hidden": 1}), "{r}");
    assert!(r["warning"].is_string(), "{r}");
    assert!(s.doc().unwrap().doc.images[&image_key(&s, a)].indexed_png());
    assert_eq!((image_key(&s, locked), image_key(&s, hidden)), ("huge".to_string(), kh));
}

/// The uniform reduction takes the most levels a channel whose cube leaves one entry for
/// transparency (3 for 64 colors, 6 for 256), and the document reduction takes 1 to 255 swatch
/// colors.
#[test]
fn convert_to_indexed_sizes_the_uniform_and_document_palettes() {
    let mut s = session();
    let id = image_of(&mut s, [[200, 30, 90, 255], [20, 40, 220, 255], [120, 130, 140, 255], [250, 250, 250, 255]]);
    s.execute("edit.colors.toIndexed", &json!({"reduction": "uniform", "colors": 64, "dither": "none"})).unwrap();
    assert_eq!(pixels(&s, id), [[255, 0, 127, 255], [0, 0, 255, 255], [127, 127, 127, 255], [255, 255, 255, 255]]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.colors.toIndexed", &json!({"reduction": "uniform", "dither": "none"})).unwrap();
    assert_eq!(pixels(&s, id), [[204, 51, 102, 255], [0, 51, 204, 255], [102, 153, 153, 255], [255, 255, 255, 255]]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.edit("No swatches", |d, _| {
        d.swatches.clear();
        d.swatch_groups.clear();
        Ok(())
    })
    .unwrap();
    let e = s.execute("edit.colors.toIndexed", &json!({"reduction": "document"})).unwrap_err().to_string();
    assert!(e.contains("the document has no color swatches"), "{e}");
    s.edit("Many swatches", |d, _| {
        for i in 0..256u32 {
            let paint = Paint::solid(Color::rgb8(i as u8, 7, 9));
            d.swatches.push(vectorcraft_color::swatch::Swatch { name: format!("c{i}"), paint, global: false, spot: false });
        }
        Ok(())
    })
    .unwrap();
    let e = s.execute("edit.colors.toIndexed", &json!({"reduction": "document"})).unwrap_err().to_string();
    assert!(e.contains("the document's swatches hold 256 colors; the document reduction takes at most 255"), "{e}");
    assert_eq!(image_key(&s, id), "pic");
}

/// `dither` and `ditherAmount` reach the quantizer: a gray ramp reduced to black and white is
/// dithered by each method, and at an amount of 0 it comes out as with no dither.
#[test]
fn convert_to_indexed_dithers_by_the_amount_given() {
    let ramp = |p: Value| {
        let mut s = session();
        let id = placed(&mut s, 16, 4, |x, _| [(x * 17) as u8, (x * 17) as u8, (x * 17) as u8, 255]);
        s.execute("edit.colors.toIndexed", &p).unwrap();
        pixels(&s, id)
    };
    let none = ramp(json!({"reduction": "blackWhite", "dither": "none"}));
    for d in ["diffusion", "pattern", "noise"] {
        assert_eq!(ramp(json!({"reduction": "blackWhite", "dither": d, "ditherAmount": 0})), none, "{d} at 0");
        assert_ne!(ramp(json!({"reduction": "blackWhite", "dither": d})), none, "{d} at 100");
    }
}

/// A PDF of a CMYK document writes an image Convert to Grayscale made with one channel (#5632).
#[test]
fn a_converted_image_is_written_to_pdf_as_gray_in_a_cmyk_document() {
    let mut s = session();
    s.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    image_of(&mut s, [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [10, 200, 30, 255]]);
    s.execute("edit.colors.toGrayscale", &json!({})).unwrap();
    let opts = vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() };
    let pdf = String::from_utf8_lossy(&vectorcraft_pdf::export(&s.doc().unwrap().doc, &opts).unwrap()).into_owned();
    let image = pdf.split("/Subtype/Image").nth(1).or_else(|| pdf.split("/Subtype /Image").nth(1)).expect("an image in the PDF");
    let image = &image[..image.find("stream").unwrap_or(image.len())];
    assert!(image.contains("/DeviceGray") && !image.contains("/DeviceRGB"), "one color channel: {image}");
}

/// "Dots": a red square tile; a selected rectangle filled with it.
fn pattern_fill(s: &mut Session) -> NodeId {
    rect(s, json!("#ff0000"));
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Dots"})).unwrap();
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    r
}

fn tile_fill(s: &Session, pattern: &str) -> String {
    let d = &s.doc().unwrap().doc;
    d.pattern(pattern).unwrap().art[0].appearance.fill_paint().color().unwrap().to_hex()
}

fn fill_pattern(s: &Session, id: NodeId) -> String {
    match s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint() {
        Paint::Pattern { pattern, .. } => pattern,
        p => panic!("not a pattern fill: {p:?}"),
    }
}

#[test]
fn recolor_writes_a_new_pattern_swatch() {
    let mut s = session();
    let r = pattern_fill(&mut s);
    let colors = s.execute("recolor.colors", &json!({})).unwrap();
    assert!(colors["colors"].as_array().unwrap().iter().any(|c| c["hex"] == "#ff0000"), "the tile's colours are listed: {colors}");
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("recolor.apply", &json!({"map": {"#ff0000": "#0000ff"}})).unwrap();
    assert_eq!(fill_pattern(&s, r), "Dots 2");
    assert_eq!(tile_fill(&s, "Dots 2"), "#0000ff");
    assert_eq!(tile_fill(&s, "Dots"), "#ff0000", "the original pattern is untouched");
    let d = &s.doc().unwrap().doc;
    assert!(d.swatch("Dots 2").is_some_and(|w| matches!(w.paint, Paint::Pattern { .. })), "a new pattern swatch");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(fill_pattern(&s, r), "Dots");
    assert!(s.doc().unwrap().doc.pattern("Dots 2").is_none());
    // Edit Colors recolour pattern fills the same way; patterns can be left out.
    s.execute("edit.colors.invert", &json!({"includePatterns": false})).unwrap();
    assert_eq!(fill_pattern(&s, r), "Dots");
    s.execute("edit.colors.invert", &json!({})).unwrap();
    assert_eq!((fill_pattern(&s, r), tile_fill(&s, "Dots 2")), ("Dots 2".to_string(), "#00ffff".to_string()));
}

#[test]
fn edit_colors_of_everything_count_the_hidden_objects_left_out() {
    let mut s = session();
    let shown = rect(&mut s, json!("#ff0000"));
    let hidden = rect(&mut s, json!("#ff0000"));
    // A hidden object with no paint would not change.
    let unpainted = rect(&mut s, json!("#ff0000"));
    s.execute("paint.setFill", &json!({"ids": [unpainted.0], "none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [unpainted.0], "none": true})).unwrap();
    s.execute("layer.setProps", &json!({"ids": [hidden.0, unpainted.0], "visible": false})).unwrap();
    // A locked member of a selected group is recolored with it.
    let member = rect(&mut s, json!("#ff0000"));
    let locked_member = rect(&mut s, json!("#ff0000"));
    s.execute("select.set", &json!({"ids": [member.0, locked_member.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("layer.setProps", &json!({"ids": [locked_member.0], "locked": true})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let r = s.execute("edit.colors.invert", &json!({})).unwrap();
    assert_eq!(r["notChanged"], json!({"locked": 0, "hidden": 1}), "{r}");
    assert_eq!(r["warning"], "Recolor completed; 1 hidden object was left unchanged");
    assert_eq!((fill_of(&s, shown).to_hex(), fill_of(&s, hidden).to_hex()), ("#00ffff".to_string(), "#ff0000".to_string()));
    assert_eq!(fill_of(&s, locked_member).to_hex(), "#00ffff");
    // Objects given by `ids` are the whole scope.
    let r = s.execute("edit.colors.toGrayscale", &json!({"ids": [shown.0]})).unwrap();
    assert_eq!(r["notChanged"], json!({"locked": 0, "hidden": 0}), "{r}");
    assert!(r.get("warning").is_none(), "{r}");
}

#[test]
fn edit_colors_in_isolation_mode_count_the_locked_members_left_out() {
    let mut s = session();
    let a = rect(&mut s, json!("#ff0000"));
    let b = rect(&mut s, json!("#ff0000"));
    let c = rect(&mut s, json!("#ff0000"));
    s.execute("select.set", &json!({"ids": [a.0, b.0, c.0]})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("layer.setProps", &json!({"ids": [b.0, c.0], "locked": true})).unwrap();
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let r = s.execute("edit.colors.saturate", &json!({"intensity": -50})).unwrap();
    assert_eq!(r["notChanged"], json!({"locked": 2, "hidden": 0}), "{r}");
    assert_eq!(r["warning"], "Recolor completed; 2 locked objects were left unchanged");
}
