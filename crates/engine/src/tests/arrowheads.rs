//! Custom arrowheads (`cmd/arrowheads.rs`): heads read from the arrowheads file's symbols, listed,
//! set with `stroke.set`, reloaded when the file is saved, and kept with the documents that use
//! them.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Arrowhead, Document, Node, StrokeLayer, Symbol, SymbolRegistration};
use vectorcraft_geom::{PathData, Point, SubPath};

use crate::cmd::arrowheads::{self, heads_of};
use crate::*;

fn polygon(pts: &[(f64, f64)], ap: Appearance) -> Node {
    let pts: Vec<Point> = pts.iter().map(|&(x, y)| Point::new(x, y)).collect();
    Node::path(NodeId(0), PathData::single(SubPath::polyline(&pts, true)), ap)
}

fn add_symbol(d: &mut Document, name: &str, art: Node) {
    let art = d.reid(&art);
    d.symbols.push(Symbol { name: name.into(), art: Arc::new(art), dynamic: false });
}

fn filled() -> Appearance {
    Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 1.0)
}

/// A folder of its own for test `name`, emptied.
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-arrowheads-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session_with_line() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let r = s.execute("shape.line", &json!({"x1": 10, "y1": 100, "x2": 150, "y2": 100})).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn stroke(s: &Session, id: NodeId) -> StrokeLayer {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().clone()
}

#[test]
fn each_symbol_is_a_head_pointing_right_with_its_tip_at_the_registration_point() {
    let mut d = Document::new(100.0, 100.0);
    // 20 high, so a unit is 5 pt: tip on the right edge's middle.
    add_symbol(&mut d, "Wedge", polygon(&[(0.0, -10.0), (20.0, 0.0), (0.0, 10.0)], filled()));
    d.set_symbol_registration("Wedge", SymbolRegistration(5));
    // A stroke alone paints its outline: a 10 × 4 band, tip at its centre (the default).
    let mut line = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 4.0);
    line.stroke_mut().unwrap().cap = vectorcraft_doc::LineCap::Butt;
    let band = Node::path(NodeId(0), PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false)), line);
    add_symbol(&mut d, "Band", band);
    // Neither filled nor stroked: no head.
    add_symbol(&mut d, "Ghost", polygon(&[(0.0, 0.0), (5.0, 0.0), (5.0, 5.0)], Appearance::default()));
    // Far longer than wide.
    add_symbol(&mut d, "Needle", polygon(&[(0.0, 0.0), (1000.0, 0.5), (0.0, 1.0)], filled()));
    // A fill at 0 % opacity paints nothing.
    let mut clear = filled();
    if let Some(vectorcraft_doc::AppearanceItem::Fill(f)) = clear.items.first_mut() {
        f.opacity = 0.0;
    }
    add_symbol(&mut d, "Clear", polygon(&[(0.0, 0.0), (5.0, 0.0), (5.0, 5.0)], clear));
    // More anchors than a symbol's art may have: skipped before its shapes are united.
    let dense: Vec<(f64, f64)> = (0..=arrowheads::MAX_ART_ANCHORS).map(|i| ((i as f64).cos(), (i as f64).sin())).collect();
    add_symbol(&mut d, "Dense", polygon(&dense, filled()));
    // A 20 × 4 bar clipped to a 4 × 4 square: a 4 × 4 head.
    let mut mask = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)], Appearance::default());
    if let NodeKind::Path { clipping, .. } = &mut mask.kind {
        *clipping = true;
    }
    let bar = polygon(&[(0.0, 0.0), (20.0, 0.0), (20.0, 4.0), (0.0, 4.0)], filled());
    add_symbol(&mut d, "Clipped", Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(mask), Arc::new(bar)], clip: true }));
    // An 8 × 4 bar grown by 2 pt on every side (Offset Path): 12 × 8, so 6 units long.
    let mut grown = filled();
    grown.effects.push(vectorcraft_doc::Effect { id: "path.offsetPath".into(), params: json!({"offset": 2}), visible: true });
    add_symbol(&mut d, "Grown", polygon(&[(0.0, 0.0), (8.0, 0.0), (8.0, 4.0), (0.0, 4.0)], grown));
    // The same effect on the fill alone.
    let mut fill_grown = filled();
    fill_grown.items[0].effects_mut().push(vectorcraft_doc::Effect { id: "path.offsetPath".into(), params: json!({"offset": 2}), visible: true });
    add_symbol(&mut d, "Fill Grown", polygon(&[(0.0, 0.0), (8.0, 0.0), (8.0, 4.0), (0.0, 4.0)], fill_grown));
    // And on a group holding the bar.
    let mut group = Node::group(NodeId(0), vec![Arc::new(polygon(&[(0.0, 0.0), (8.0, 0.0), (8.0, 4.0), (0.0, 4.0)], filled()))]);
    group.appearance.effects.push(vectorcraft_doc::Effect { id: "path.offsetPath".into(), params: json!({"offset": 2}), visible: true });
    add_symbol(&mut d, "Group Grown", group);
    // A clipping path stroked 2 pt: its stroke paints unclipped and does not widen the clip.
    let mut frame = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)], Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0));
    if let NodeKind::Path { clipping, .. } = &mut frame.kind {
        *clipping = true;
    }
    let bar = polygon(&[(0.0, 0.0), (20.0, 0.0), (20.0, 4.0), (0.0, 4.0)], filled());
    add_symbol(&mut d, "Framed", Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(frame), Arc::new(bar)], clip: true }));
    // A group's own fill paints its unpainted 8 × 4 member.
    let mut painted_group = Node::group(NodeId(0), vec![Arc::new(polygon(&[(0.0, 0.0), (8.0, 0.0), (8.0, 4.0), (0.0, 4.0)], Appearance::default()))]);
    painted_group.appearance = filled().into();
    add_symbol(&mut d, "Group Fill", painted_group);
    // A 10 × 4 band stroked with a gradient along it.
    let gradient = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default())));
    let mut along = Appearance::basic(Paint::None, gradient, 4.0);
    if let Some(st) = along.stroke_mut() {
        (st.cap, st.gradient_mode) = (vectorcraft_doc::LineCap::Butt, vectorcraft_doc::StrokeGradientMode::Along);
    }
    let band = Node::path(NodeId(0), PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false)), along);
    add_symbol(&mut d, "Gradient Band", band);
    // An 8 × 4 mesh of two patches whose right one is fully transparent: a 4 × 4 head.
    let corners = vectorcraft_doc::live::grid_points(vectorcraft_geom::Rect::new(0.0, 0.0, 8.0, 4.0), 1, 2);
    let points = corners
        .iter()
        .map(|&p| vectorcraft_doc::MeshPoint { p, color: Color::BLACK, opacity: if p.x > 1.0 { 0.0 } else { 1.0 }, handles: Default::default() })
        .collect();
    let mesh = vectorcraft_doc::GradientMesh { rows: 1, cols: 2, points };
    add_symbol(&mut d, "Half Mesh", Node::new(NodeId(0), NodeKind::Mesh(mesh)));
    // An effect on the file's layer is not part of any symbol.
    Arc::make_mut(&mut d.layers[0]).appearance.effects.push(vectorcraft_doc::Effect {
        id: "path.offsetPath".into(),
        params: json!({"offset": 5}),
        visible: true,
    });
    let read = heads_of(&d);
    let names: Vec<&str> = read.heads.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, ["Wedge", "Band", "Clipped", "Grown", "Fill Grown", "Group Grown", "Framed", "Group Fill", "Gradient Band", "Half Mesh"]);
    let b = read.heads[9].path.bounds().unwrap();
    assert!((b.width() - 4.0).abs() < 1e-3 && (b.height() - 4.0).abs() < 1e-3, "the mesh's painted patch: {b:?}");
    let b = read.heads[7].path.bounds().unwrap();
    assert!((b.width() - 8.0).abs() < 1e-3 && (b.height() - 4.0).abs() < 1e-3, "the group's fill: {b:?}");
    let (g, plain) = (read.heads[8].path.bounds().unwrap(), read.heads[1].path.bounds().unwrap());
    let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
    assert!(near(g.x0, plain.x0) && near(g.x1, plain.x1) && near(g.y0, plain.y0) && near(g.y1, plain.y1), "the gradient stroke's outline: {g:?}");
    let framed = read.heads[6].path.to_bezpath();
    let b = vectorcraft_geom::Shape::bounding_box(&framed);
    assert!((b.width() - 4.0).abs() < 1e-3 && (b.height() - 4.0).abs() < 1e-3, "6 × 6 with the stroke: {b:?}");
    assert_ne!(vectorcraft_geom::Shape::winding(&framed, Point::new(0.1, 0.1)), 0, "the clipped bar fills the middle");
    for h in &read.heads[3..6] {
        let b = h.path.bounds().unwrap();
        assert!((b.width() - 6.0).abs() < 1e-3 && (b.height() - 4.0).abs() < 1e-3, "{}: the effect's outline: {b:?}", h.name);
    }
    let skipped: Vec<&str> = read.skipped.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(skipped, ["Ghost", "Needle", "Clear", "Dense"], "{:?}", read.skipped);
    let b = read.heads[2].path.bounds().unwrap();
    assert!((b.width() - 4.0).abs() < 1e-3 && (b.height() - 4.0).abs() < 1e-3, "clipped to the square: {b:?}");
    let b = read.heads[0].path.bounds().unwrap();
    assert!(b.x0.abs() < 1e-6 && (b.x1 - 4.0).abs() < 1e-6 && (b.y0 + 2.0).abs() < 1e-6 && (b.y1 - 2.0).abs() < 1e-6, "{b:?}");
    assert!((read.heads[0].reach() - 4.0f64.hypot(2.0)).abs() < 1e-6);
    let b = read.heads[1].path.bounds().unwrap();
    assert!((b.x0 + 5.0).abs() < 1e-3 && (b.x1 - 5.0).abs() < 1e-3 && (b.y0 + 2.0).abs() < 1e-3 && (b.y1 - 2.0).abs() < 1e-3, "{b:?}");
}

#[test]
fn the_arrowheads_file_is_made_opened_and_reloaded_when_saved() {
    let dir = folder("edit");
    let file = dir.join(arrowheads::FILE_NAME).to_string_lossy().to_string();
    let (mut s, id) = session_with_line();
    s.attach_arrowheads_file(file.clone());
    let list = s.execute("stroke.arrowheads.list", &json!({})).unwrap();
    assert_eq!((list["arrowheads"].as_array().unwrap().len(), list["file"].as_str()), (0, Some(file.as_str())));
    assert!(s.execute("stroke.set", &json!({"endArrow": {"custom": "Example Arrowhead"}})).is_err(), "no such head yet");
    let line_doc = s.doc().unwrap().uid;
    let r = s.execute("stroke.arrowheads.edit", &json!({})).unwrap();
    assert_eq!(r["created"], true);
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(file.as_str()), "the file is the active document");
    let list = s.execute("stroke.arrowheads.list", &json!({})).unwrap();
    assert_eq!(list["arrowheads"][0]["name"], "Example Arrowhead");
    // A second head drawn in the file is listed once the file is saved.
    let st = s.doc_mut().unwrap();
    let d = Arc::make_mut(&mut st.doc);
    add_symbol(d, "Square", polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)], filled()));
    assert_eq!(s.execute("stroke.arrowheads.list", &json!({})).unwrap()["arrowheads"].as_array().unwrap().len(), 1);
    s.execute("document.save", &json!({})).unwrap();
    let list = s.execute("stroke.arrowheads.list", &json!({})).unwrap();
    let names: Vec<&str> = list["arrowheads"].as_array().unwrap().iter().filter_map(|h| h["name"].as_str()).collect();
    assert_eq!(names, ["Example Arrowhead", "Square"]);
    // Editing again from the line's document brings the open file to the front, without making
    // it anew or opening it twice.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(s.doc().unwrap().uid, line_doc);
    assert_eq!(s.execute("stroke.arrowheads.edit", &json!({})).unwrap()["created"], false);
    assert_eq!((s.documents().len(), s.doc().unwrap().path.as_deref()), (2, Some(file.as_str())));
    // The line takes the head with its art.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    s.execute("stroke.set", &json!({"ids": [id.0], "endArrow": {"custom": "Square"}})).unwrap();
    let Some(Arrowhead::Custom(c)) = stroke(&s, id).end_arrow else { panic!("a custom head") };
    assert_eq!(c.name, "Square");
    assert!(s.execute("stroke.set", &json!({"endArrow": {"custom": 3}})).is_err());
    assert!(s.execute("stroke.set", &json!({"endArrow": {"custom": "Circle"}})).is_err());
    // A head given with its art is refused when the art could not be drawn.
    let mut far = serde_json::to_value(Arrowhead::Custom(c.clone())).unwrap();
    far["Custom"]["path"] = serde_json::to_value(PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(1e6, 1.0)], true))).unwrap();
    assert!(s.execute("stroke.set", &json!({"ids": [id.0], "endArrow": far})).is_err());
    let mut empty = serde_json::to_value(Arrowhead::Custom(c)).unwrap();
    empty["Custom"]["path"] = json!({"subpaths": []});
    assert!(s.execute("stroke.set", &json!({"ids": [id.0], "endArrow": empty})).is_err());
    // One anchor, or anchors on one line, fill nothing.
    for pts in [vec![Point::new(1.0, 1.0)], vec![Point::new(0.0, 0.0), Point::new(2.0, 0.0), Point::new(4.0, 0.0)]] {
        let mut flat =
            serde_json::to_value(Arrowhead::Custom(Arc::new(vectorcraft_doc::CustomArrowhead::new("Flat".into(), PathData::default())))).unwrap();
        flat["Custom"]["path"] = serde_json::to_value(PathData::single(SubPath::polyline(&pts, false))).unwrap();
        assert!(s.execute("stroke.set", &json!({"ids": [id.0], "endArrow": flat})).is_err(), "{pts:?}");
    }
    // A lone anchor draws no segment, whatever its handles.
    let mut lone = vectorcraft_geom::Anchor::corner(Point::new(1.0, 0.0));
    (lone.h_in, lone.h_out) = (Point::new(0.0, 1.0), Point::new(2.0, -1.0));
    let mut handles =
        serde_json::to_value(Arrowhead::Custom(Arc::new(vectorcraft_doc::CustomArrowhead::new("Lone".into(), PathData::default())))).unwrap();
    handles["Custom"]["path"] = serde_json::to_value(PathData::single(SubPath::new(vec![lone], true))).unwrap();
    assert!(s.execute("stroke.set", &json!({"ids": [id.0], "endArrow": handles.clone()})).is_err(), "a lone anchor");
    // Two equal squares winding opposite ways cancel out.
    let sq = [Point::new(0.0, -1.0), Point::new(2.0, -1.0), Point::new(2.0, 1.0), Point::new(0.0, 1.0)];
    let back: Vec<Point> = sq.iter().rev().copied().collect();
    let cancel = PathData::new(vec![SubPath::polyline(&sq, true), SubPath::polyline(&back, true)]);
    handles["Custom"]["path"] = serde_json::to_value(cancel).unwrap();
    assert!(s.execute("stroke.set", &json!({"ids": [id.0], "endArrow": handles})).is_err(), "opposite squares");
    // A bow tie's lobes wind opposite ways, so its signed area is 0, and it still fills both.
    let tie = [Point::new(0.0, -1.0), Point::new(2.0, 1.0), Point::new(2.0, -1.0), Point::new(0.0, 1.0)];
    let mut bow =
        serde_json::to_value(Arrowhead::Custom(Arc::new(vectorcraft_doc::CustomArrowhead::new("Bow".into(), PathData::default())))).unwrap();
    bow["Custom"]["path"] = serde_json::to_value(PathData::single(SubPath::polyline(&tie, true))).unwrap();
    assert!(matches!(stroke(&s, id).end_arrow, Some(Arrowhead::Custom(h)) if h.name == "Square"), "the head stays as it was");
    s.execute("stroke.set", &json!({"ids": [id.0], "endArrow": bow.clone()})).unwrap();
    assert!(matches!(stroke(&s, id).end_arrow, Some(Arrowhead::Custom(h)) if h.name == "Bow"));
    // So does a very thin bar.
    let bar = [Point::new(0.0, 0.0), Point::new(4.0, 0.0), Point::new(4.0, 0.001), Point::new(0.0, 0.001)];
    bow["Custom"]["path"] = serde_json::to_value(PathData::single(SubPath::polyline(&bar, true))).unwrap();
    s.execute("stroke.set", &json!({"ids": [id.0], "endArrow": bow})).unwrap();
    let Some(Arrowhead::Custom(h)) = stroke(&s, id).end_arrow else { panic!("a custom head") };
    assert_eq!(h.path.anchor_count(), 4, "the thin bar");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_document_keeps_its_custom_heads_without_the_arrowheads_file() {
    let (mut s, id) = session_with_line();
    let mut d = Document::new(100.0, 100.0);
    add_symbol(&mut d, "Wedge", polygon(&[(0.0, -10.0), (20.0, 0.0), (0.0, 10.0)], filled()));
    let head = Arrowhead::Custom(Arc::new(heads_of(&d).heads.remove(0)));
    // As document.inspect reports it, so it reads back unchanged.
    s.execute("stroke.set", &json!({"startArrow": head, "endArrow": "Triangle"})).unwrap();
    assert_eq!(stroke(&s, id).start_arrow, Some(head.clone()));
    let inspect = s.execute("document.inspect", &json!({})).unwrap().to_string();
    assert!(inspect.contains("\"Wedge\""), "inspect names the head");
    let ink = |s: &Session| {
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &Default::default())
    };
    let with_head = ink(&s);
    // Saved and opened in a session that has no arrowheads file: the head is drawn the same.
    let saved = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].clone();
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "a.vectorcraft", "dataBase64": saved})).unwrap();
    assert_eq!(stroke(&o, id).start_arrow, Some(head));
    assert_eq!(ink(&o).pixels, with_head.pixels);
    // SVG and PDF write it as they write the built-in heads.
    let svg = |s: &mut Session| s.execute("document.export", &json!({"format": "svg"})).unwrap()["dataBase64"].as_str().unwrap().len();
    let headed = svg(&mut o);
    o.execute("stroke.set", &json!({"ids": [id.0], "startArrow": null})).unwrap();
    assert!(headed > svg(&mut o), "the head is in the SVG");
    // Undo puts it back.
    o.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(stroke(&o, id).start_arrow, Some(Arrowhead::Custom(_))));
}

#[test]
fn an_unreadable_arrowheads_file_lists_no_heads_and_says_why() {
    let dir = folder("bad");
    let file = dir.join(arrowheads::FILE_NAME);
    std::fs::write(&file, b"not a drawing").unwrap();
    let mut s = Session::new();
    s.attach_arrowheads_file(file.to_string_lossy().to_string());
    let list = s.execute("stroke.arrowheads.list", &json!({})).unwrap();
    assert!(list["arrowheads"].as_array().unwrap().is_empty());
    assert!(list["error"].is_string(), "{list}");
    assert!(s.execute("stroke.arrowheads.reload", &json!({})).is_err());
    // A folder in its place is not read either.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(s.execute("stroke.arrowheads.reload", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
    // Without a file (the web, runs without preferences) there is nothing to edit.
    let mut bare = Session::new();
    assert!(bare.execute("stroke.arrowheads.edit", &json!({})).is_err());
    assert_eq!(bare.execute("stroke.arrowheads.list", &json!({})).unwrap()["file"], serde_json::Value::Null);
}

#[test]
fn the_heads_listed_after_a_save_are_read_from_the_file_and_editing_keeps_it() {
    let dir = folder("saved");
    let file = dir.join(arrowheads::FILE_NAME).to_string_lossy().to_string();
    let (mut s, _) = session_with_line();
    s.attach_arrowheads_file(file.clone());
    assert_eq!(s.execute("stroke.arrowheads.edit", &json!({})).unwrap()["created"], true);
    // A speck that saving rounds to a point is a head only before the save.
    let st = s.doc_mut().unwrap();
    add_symbol(Arc::make_mut(&mut st.doc), "Speck", polygon(&[(0.0, 0.0), (0.3, 0.1), (0.0, 0.2)], filled()));
    assert!(heads_of(&s.doc().unwrap().doc).heads.iter().any(|h| h.name == "Speck"), "a head before rounding");
    s.execute("document.save", &json!({"precision": 0})).unwrap();
    let list = s.execute("stroke.arrowheads.list", &json!({})).unwrap();
    let names: Vec<&str> = list["arrowheads"].as_array().unwrap().iter().filter_map(|h| h["name"].as_str()).collect();
    assert_eq!(names, ["Example Arrowhead"]);
    assert_eq!(list["skipped"][0]["name"], "Speck", "{list}");
    // Editing again leaves the file as it is.
    let saved = std::fs::read(&file).unwrap();
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(s.execute("stroke.arrowheads.edit", &json!({})).unwrap()["created"], false);
    assert_eq!(std::fs::read(&file).unwrap(), saved, "the file is left as it was");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_arrowheads_file_that_unpacks_past_the_size_limit_is_not_read() {
    let dir = folder("unpacked");
    let file = dir.join(arrowheads::FILE_NAME);
    let mut d = arrowheads::starter();
    d.title = "a".repeat(arrowheads::MAX_FILE_BYTES as usize);
    let bytes = vectorcraft_format::save_with(&d, &vectorcraft_format::SaveOptions { compress: true, ..Default::default() }).unwrap();
    assert!((bytes.len() as u64) < arrowheads::MAX_FILE_BYTES / 16, "compressed small: {}", bytes.len());
    std::fs::write(&file, &bytes).unwrap();
    let mut s = Session::new();
    s.attach_arrowheads_file(file.to_string_lossy().to_string());
    let list = s.execute("stroke.arrowheads.list", &json!({})).unwrap();
    assert!(list["arrowheads"].as_array().unwrap().is_empty(), "{}", list["arrowheads"]);
    assert!(list["error"].is_string(), "{}", list["error"]);
    // The same head in a file of an ordinary size is read.
    std::fs::write(&file, vectorcraft_format::save_file(&arrowheads::starter())).unwrap();
    assert_eq!(s.execute("stroke.arrowheads.reload", &json!({})).unwrap()["count"], 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_symbol_whose_shapes_cannot_be_combined_is_skipped() {
    let wedge = || Arc::new(polygon(&[(0.0, -10.0), (20.0, 0.0), (0.0, 10.0)], filled()));
    // Too far out for the shape operations to measure: the union would leave it out.
    let far = || Arc::new(polygon(&[(0.0, 0.0), (1e200, 0.0), (0.0, 1e200)], filled()));
    let mut d = Document::new(100.0, 100.0);
    add_symbol(&mut d, "Wedge", (*wedge()).clone());
    add_symbol(&mut d, "Broken", Node::group(NodeId(0), vec![wedge(), far()]));
    // The same as clipped art.
    let mut mask = polygon(&[(0.0, -10.0), (20.0, -10.0), (20.0, 10.0), (0.0, 10.0)], Appearance::default());
    if let NodeKind::Path { clipping, .. } = &mut mask.kind {
        *clipping = true;
    }
    add_symbol(&mut d, "Broken Clip", Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(mask), wedge(), far()], clip: true }));
    let read = heads_of(&d);
    let names: Vec<&str> = read.heads.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, ["Wedge"]);
    let skipped: Vec<&str> = read.skipped.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(skipped, ["Broken", "Broken Clip"], "{:?}", read.skipped);
}
