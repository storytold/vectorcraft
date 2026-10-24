//! Edit → Edit Colors: Adjust Color Balance modes and the live preview.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::Node;
use vectorcraft_tools::{PointerEvent, PointerKind};

use super::*;
use crate::tooling::ViewInfo;

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
    let bytes = png([[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 128]]);
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
