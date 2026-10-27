//! Envelope Distort: Reset with Warp / Mesh, Envelope Options, Release, Expand and Object → Expand,
//! driven through `Session::execute`.

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::live::{self, EnvelopeKind, EnvelopeOptions, PreserveShape};
use vectorcraft_doc::{Knockout, Node, NodeKind, OpacityMask};
use vectorcraft_geom::Rect;

use crate::*;

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

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

/// A 200×100 rectangle at (100, 100) enveloped by `cmd` with `params` → (rectangle, envelope).
fn enveloped(s: &mut Session, cmd: &str, params: Value) -> (NodeId, NodeId) {
    let a = rect(s, 100.0, 100.0, 200.0, 100.0);
    sel(s, &[a]);
    (a, id_of(&s.execute(cmd, &params).unwrap()))
}

fn kind(s: &Session, id: NodeId) -> EnvelopeKind {
    match node(s, id).kind {
        NodeKind::Envelope { kind, .. } => *kind,
        k => panic!("{k:?}"),
    }
}

fn options(s: &Session, id: NodeId) -> (EnvelopeOptions, f64) {
    match node(s, id).kind {
        NodeKind::Envelope { options, fidelity, .. } => (options, fidelity),
        k => panic!("{k:?}"),
    }
}

/// `bytes` without the lines that depend on when the file was written: the EPS and PDF dates, and
/// a PDF's XMP packet (one line, with its dates and IDs) and trailer `/ID`.
fn undated(bytes: &[u8]) -> Vec<u8> {
    let stamps: [&[u8]; 5] = [b"CreationDate", b"ModDate", b"xmp:CreateDate", b"xmp:ModifyDate", b"/ID["];
    let dated = |line: &&[u8]| stamps.iter().any(|s| line.windows(s.len()).any(|w| w == *s));
    bytes.split(|b| *b == b'\n').filter(|l| !dated(l)).flat_map(|l| l.iter().copied().chain(*b"\n")).collect()
}

#[test]
fn new_envelopes_take_the_reference_defaults_and_options_change_them() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({}));
    assert_eq!(options(&s, e), (EnvelopeOptions::NEW, 50.0));
    s.execute(
        "object.envelope.options",
        &json!({"antiAlias": false, "preserveShape": "transparency", "distortAppearance": true, "distortLinearGradients": true, "distortPatternFills": true, "fidelity": 75}),
    )
    .unwrap();
    let (o, f) = options(&s, e);
    assert_eq!((o.anti_alias, o.preserve_shape, o.gradients(), o.patterns(), f), (false, PreserveShape::Transparency, true, true, 75.0));
    let info = s.execute("object.envelope.info", &json!({})).unwrap();
    assert_eq!(info["id"], json!(e.0));
    assert_eq!((info["type"].clone(), info["style"].clone(), info["preserveShape"].clone()), (json!("warp"), json!("arc"), json!("transparency")));
    assert!(s.execute("object.envelope.options", &json!({"preserveShape": "alpha"})).is_err());
    assert!(s.execute("object.envelope.options", &json!({"fidelity": -1})).is_err());
}

#[test]
fn options_with_nothing_selected_are_the_defaults_for_new_envelopes() {
    let mut s = session();
    s.execute("object.envelope.options", &json!({"distortAppearance": false, "fidelity": 20})).unwrap();
    let info = s.execute("object.envelope.info", &json!({})).unwrap();
    assert_eq!((info["id"].clone(), info["distortAppearance"].clone(), info["fidelity"].clone()), (Value::Null, json!(false), json!(20.0)));
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 2, "cols": 3}));
    let (o, f) = options(&s, e);
    assert_eq!((o.distort_appearance, f), (false, 20.0));
    assert!(matches!(kind(&s, e), EnvelopeKind::Mesh { rows: 2, cols: 3, .. }));
}

#[test]
fn reset_with_warp_and_mesh_switch_the_kind_and_keep_the_content() {
    let mut s = session();
    let (a, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 2, "cols": 2}));
    s.execute("object.envelope.resetWithWarp", &json!({"style": "arch", "bend": 60, "orientation": "vertical"})).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Warp { ref style, bend, horizontal: false, .. } if style == "arch" && bend == 60.0));
    assert_eq!(node(&s, e).children().unwrap()[0].id, a);
    // Unset values keep the current warp's.
    s.execute("object.envelope.resetWithWarp", &json!({"bend": -20})).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Warp { ref style, bend, horizontal: false, .. } if style == "arch" && bend == -20.0));
    assert!(s.execute("object.envelope.resetWithWarp", &json!({"style": "nope"})).is_err());
    // Back to a mesh that keeps the arch's shape: the top middle point is where the warp put it.
    s.execute("object.envelope.resetWithWarp", &json!({"style": "arch", "bend": 50, "horizontal": true})).unwrap();
    let top = live::EnvelopeMap::of(&node(&s, e)).unwrap().surface_mesh(2, 2, vectorcraft_color::Color::BLACK);
    s.execute("object.envelope.resetWithMesh", &json!({"rows": 2, "cols": 2})).unwrap();
    let EnvelopeKind::Mesh { rows: 2, cols: 2, points, .. } = kind(&s, e) else { panic!("{:?}", kind(&s, e)) };
    assert!(points[1].distance(top.points[1].p) < 1e-9 && points[1].y < 99.0, "{:?}", points[1]);
    // Without Maintain Envelope Shape: a flat grid over the content.
    s.execute("object.envelope.resetWithMesh", &json!({"rows": 1, "cols": 1, "maintainShape": false})).unwrap();
    assert_eq!(
        kind(&s, e),
        EnvelopeKind::Mesh { rows: 1, cols: 1, points: live::grid_points(Rect::new(100.0, 100.0, 300.0, 200.0), 1, 1), handles: vec![] }
    );
    assert!(s.execute("object.envelope.resetWithMesh", &json!({"rows": 0})).is_err());
    // One undo step per reset.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Mesh { rows: 2, cols: 2, .. }));
    // Nothing to reset without an envelope.
    sel(&mut s, &[]);
    assert!(s.execute("object.envelope.resetWithMesh", &json!({})).is_err());
}

#[test]
fn release_gives_back_the_warp_shape_as_a_mesh_and_keeps_the_transparency() {
    let mut s = session();
    let (a, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "arc", "bend": 50}));
    s.execute("transparency.set", &json!({"opacity": 40, "blend": "multiply"})).unwrap();
    let mask = Node::path(NodeId(0), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Default::default());
    s.edit("mask", |d, _| {
        d.node_mut(e).unwrap().extras_mut().mask = Some(Box::new(OpacityMask::new(mask, true)));
        Ok(())
    })
    .unwrap();
    let r = s.execute("object.envelope.release", &json!({})).unwrap();
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], a);
    let content = node(&s, a);
    assert_eq!((content.opacity, content.blend, content.extras().mask.is_some()), (0.4, BlendMode::Multiply, true), "the content keeps the look");
    let NodeKind::Mesh(m) = node(&s, ids[1]).kind else { panic!("the warp's shape comes back as a mesh") };
    // The mesh follows the arc: its outline reaches below the content's bottom edge at the sides.
    let b = m.outline().bounds().unwrap();
    assert!(b.y1 > 201.0, "{b:?}");
    assert_eq!(s.doc().unwrap().selection.objects, ids);
}

#[test]
fn release_groups_several_objects_under_the_transparency() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let b = rect(&mut s, 200.0, 100.0, 50.0, 50.0);
    sel(&mut s, &[a, b]);
    let e = id_of(&s.execute("object.envelope.makeWithMesh", &json!({"rows": 1, "cols": 2})).unwrap());
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let r = s.execute("object.envelope.release", &json!({})).unwrap();
    let g = node(&s, NodeId(r["ids"][0].as_u64().unwrap()));
    assert_eq!(g.opacity, 0.5);
    assert_eq!(g.children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), vec![a, b]);
    assert!(matches!(node(&s, NodeId(r["ids"][1].as_u64().unwrap())).kind, NodeKind::Mesh(ref m) if (m.rows, m.cols) == (1, 2)));
    assert!(s.doc().unwrap().doc.node(e).is_none());
}

#[test]
fn expand_keeps_name_transparency_knockout_and_mask() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "bulge", "bend": 40}));
    s.execute("transparency.set", &json!({"opacity": 70, "isolate": true, "knockout": "on"})).unwrap();
    let mask = Node::path(NodeId(0), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Default::default());
    s.edit("name and mask", |d, _| {
        let n = d.node_mut(e).unwrap();
        n.name = Some("Banner".into());
        n.extras_mut().mask = Some(Box::new(OpacityMask::new(mask, true)));
        Ok(())
    })
    .unwrap();
    s.execute("object.envelope.expand", &json!({})).unwrap();
    let g = node(&s, e);
    assert!(matches!(g.kind, NodeKind::Group { .. }));
    assert_eq!((g.name.as_deref(), g.opacity, g.isolate, g.knockout, g.extras().mask.is_some()), (Some("Banner"), 0.7, true, Knockout::On, true));
}

#[test]
fn object_expand_expands_envelopes_inside_the_selection() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "arch", "bend": 50}));
    let other = rect(&mut s, 400.0, 400.0, 20.0, 20.0);
    sel(&mut s, &[e, other]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let info = s.execute("object.expand.info", &json!({})).unwrap();
    assert_eq!(info["object"], json!(true));
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let mut envelopes = 0;
    node(&s, g).walk(&mut |n| envelopes += matches!(n.kind, NodeKind::Envelope { .. }) as usize);
    assert_eq!(envelopes, 0);
    // The arch lifted the expanded art's middle above the rectangle's top edge.
    let b = node(&s, e).geometric_bounds().unwrap();
    assert!(b.y0 < 99.0, "{b:?}");
}

#[test]
fn envelope_options_round_trip_natively_and_old_files_load() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({}));
    s.execute("object.envelope.options", &json!({"preserveShape": "transparency", "distortPatternFills": true})).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(back.node(e), d.node(e));
    // An envelope saved before the options existed loads with its old behaviour.
    let mut old = serde_json::to_value(node(&s, e)).unwrap();
    old["kind"].as_object_mut().unwrap().remove("options");
    let n: Node = serde_json::from_value(old).unwrap();
    assert!(matches!(n.kind, NodeKind::Envelope { options, .. } if options == EnvelopeOptions::default() && !options.distort_appearance));
    // The old behaviour's options aren't written.
    let mut legacy = node(&s, e);
    if let NodeKind::Envelope { options, .. } = &mut legacy.kind {
        *options = EnvelopeOptions::default();
    }
    assert!(serde_json::to_value(&legacy).unwrap()["kind"].get("options").is_none());
}

/// Regression: every exporter wrote type inside an envelope undistorted (no text outliner), so a
/// warped line of type exported the same whatever its bend.
#[test]
fn enveloped_type_exports_distorted_in_every_format() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Envelope", "size": 48})).unwrap());
    sel(&mut s, &[t]);
    s.execute("object.envelope.makeWithWarp", &json!({"style": "arch", "bend": 0})).unwrap();
    // The export's bytes without its dates and file ID (EPS and PDF date their files to the second).
    let export = |s: &mut Session, format: &str| -> Vec<u8> {
        let data = s.execute("document.export", &json!({"format": format})).unwrap()["dataBase64"].as_str().unwrap().to_string();
        undated(&vectorcraft_format::base64_decode(&data).unwrap())
    };
    // Two PDFs dated a second apart, with the options and editing data that document.export
    // passes the writer, differ only in lines that `undated` drops.
    let d = &s.doc().unwrap().doc;
    let pdf = |at: i64| {
        let o = cmd::fileio::pdf::options("document.export", d, &json!({})).unwrap();
        undated(&export_pdf(d, &vectorcraft_pdf::PdfOptions { created: Some(at), native: Some(vectorcraft_format::save(d, false)), ..o }).unwrap())
    };
    assert_eq!(pdf(0), pdf(1), "a PDF dated a second later differs only in its dates and file ID");
    for format in ["svg", "pdf", "eps", "emf", "dxf"] {
        s.execute("object.envelope.options", &json!({"bend": 0})).unwrap();
        let flat = export(&mut s, format);
        assert_eq!(export(&mut s, format), flat, "{format} exports are repeatable");
        s.execute("object.envelope.options", &json!({"bend": 80})).unwrap();
        assert_ne!(export(&mut s, format), flat, "{format}: the bend shows in the export");
    }
    let svg = String::from_utf8(export(&mut s, "svg")).unwrap();
    assert!(!svg.contains("<text"), "type in an envelope exports as its distorted outlines");
}

/// Regression: on the canvas, type in an envelope was outlined in its first run's paint only.
#[test]
fn enveloped_type_keeps_each_runs_colour_on_the_canvas() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "MMMMMMMM", "size": 60})).unwrap());
    s.edit("red half", |d, _| {
        if let NodeKind::Text(tx) = &mut d.node_mut(t).unwrap().kind {
            vectorcraft_text::edit::style_range(&mut tx.runs, 0, 4, |st| {
                st.fill = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0))
            });
        }
        Ok(())
    })
    .unwrap();
    sel(&mut s, &[t]);
    s.execute("object.envelope.makeWithWarp", &json!({"style": "flag", "bend": 0})).unwrap();
    let b = node(&s, t).geometric_bounds().unwrap();
    let img = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 800.0, 400.0), 1.0, true);
    // Inked pixels of each half of the line: red at the start, black at the end.
    let (mut red, mut black) = (0, 0);
    for y in (b.y0 as u32)..(b.y1 as u32) {
        for x in (b.x0 as u32)..(b.x1 as u32) {
            let p = img.pixel(x, y);
            let left = (x as f64) < b.center().x;
            if p[0] > 200 && p[1] < 60 {
                assert!(left, "red at ({x}, {y})");
                red += 1;
            } else if p[0] < 60 && p[1] < 60 && p[3] > 200 {
                assert!(!left, "black at ({x}, {y})");
                black += 1;
            }
        }
    }
    assert!(red > 100 && black > 100, "{red} red, {black} black");
}

/// #1738: type warped by an envelope inside a symbol looks the same after a native save and
/// reopen: the type in symbol definitions gets its layout bounds back (they aren't saved), so the
/// envelope is fitted to the same box as before.
#[test]
fn warped_type_in_a_symbol_reopens_unchanged() {
    let mut s = session();
    s.execute("text.create", &json!({"x": 30, "y": 160, "text": "STUDIO", "size": 72, "color": "#E3AB49"})).unwrap();
    s.execute("object.envelope.makeWithWarp", &json!({"style": "wave", "bend": 18})).unwrap();
    s.execute("symbol.new", &json!({"name": "Master"})).unwrap();
    let render = |s: &Session| vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 400, 300, Affine::IDENTITY, &Default::default());
    let before = render(&s);
    let dir = std::env::temp_dir().join(format!("vectorcraft-1738-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("warped.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    let mut o = Session::new();
    o.execute("document.open", &json!({"path": path})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let d = &o.doc().unwrap().doc;
    let mut texts = 0;
    d.symbols[0].art.walk(&mut |n| {
        if let NodeKind::Text(t) = &n.kind {
            texts += 1;
            assert!(t.cached_bounds.is_some(), "the symbol's type has no layout bounds after reopening");
        }
    });
    assert_eq!(texts, 1);
    let after = render(&o);
    let differ = (0..300).flat_map(|y| (0..400).map(move |x| (x, y))).filter(|&(x, y)| before.pixel(x, y) != after.pixel(x, y)).count();
    assert_eq!(differ, 0, "{differ} pixels changed on reopening");
}
