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

/// `object.mesh.addLine` at a point a mesh envelope already has (one stored without handles) adds
/// no line and records no undo step, with or without a `color`; a point between its lines adds
/// them as one step.
#[test]
fn mesh_envelope_add_line_at_a_mesh_point_records_no_undo_step() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 2, "cols": 2}));
    let before = kind(&s, e);
    assert!(matches!(&before, EnvelopeKind::Mesh { handles, .. } if handles.is_empty()), "{before:?}");
    let undo_len = |s: &Session| s.doc().unwrap().history.undo.len();
    let n = undo_len(&s);
    for (x, y) in [(100.0, 100.0), (300.0, 200.0), (200.0, 150.0), (300.0, 150.0)] {
        s.execute("object.mesh.addLine", &json!({"id": e.0, "x": x, "y": y})).unwrap();
        assert_eq!((kind(&s, e), undo_len(&s)), (before.clone(), n), "({x}, {y}): nothing changed");
    }
    // An envelope's points carry no color, so a color there changes nothing either.
    s.execute("object.mesh.addLine", &json!({"id": e.0, "x": 200, "y": 150, "color": "#00ff00"})).unwrap();
    assert_eq!((kind(&s, e), undo_len(&s)), (before.clone(), n), "a color changes nothing");
    s.execute("object.mesh.addLine", &json!({"id": e.0, "x": 150, "y": 125})).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Mesh { rows: 3, cols: 3, .. }), "{:?}", kind(&s, e));
    assert_eq!(undo_len(&s), n + 1);
}

/// A new warp envelope bends with its style's Bézier net (`version` [`live::WARP_NET`]) and keeps
/// it through Reset with Warp. One saved before the version existed keeps the earlier map when its
/// warp or its options change.
#[test]
fn new_warp_envelopes_take_the_net_and_saved_ones_keep_their_map() {
    // The surface point a quarter of the way along the top edge.
    let quarter = |s: &Session, e| live::EnvelopeMap::of(&node(s, e)).unwrap().surface_mesh(4, 4, vectorcraft_color::Color::BLACK).points[1].p;
    let at = |x, y| vectorcraft_geom::Point::new(x, y);
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "flag", "bend": 40}));
    assert!(matches!(kind(&s, e), EnvelopeKind::Warp { version: live::WARP_NET, .. }), "{:?}", kind(&s, e));
    assert_eq!(s.execute("object.envelope.info", &json!({})).unwrap()["version"], json!(live::WARP_NET));
    // Flag moves the top edge 6 × bend × height × u(1 - u)(1 - 2u) down at u: 22.5 at u = 1/4.
    assert!(quarter(&s, e).distance(at(150.0, 122.5)) < 1e-9, "{:?}", quarter(&s, e));
    s.execute("object.envelope.resetWithWarp", &json!({"bend": -40})).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Warp { version: live::WARP_NET, .. }), "{:?}", kind(&s, e));
    assert!(quarter(&s, e).distance(at(150.0, 77.5)) < 1e-9, "{:?}", quarter(&s, e));
    // As a file saved before the version existed gives it.
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    if let Some(NodeKind::Envelope { kind, .. }) = doc.node_mut(e).map(|n| &mut n.kind)
        && let EnvelopeKind::Warp { version, .. } = &mut **kind
    {
        *version = 0;
    }
    s.execute("object.envelope.options", &json!({"fidelity": 75})).unwrap();
    s.execute("object.envelope.resetWithWarp", &json!({"bend": 40})).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Warp { version: 0, .. }), "{:?}", kind(&s, e));
    // The earlier map's flag: half the bend times sin(π x), x = -1/2 here, in half heights.
    assert!(quarter(&s, e).distance(at(150.0, 90.0)) < 1e-9, "{:?}", quarter(&s, e));
    // `version` moves a warp to a map, and a new warp can take the earlier one.
    s.execute("object.envelope.options", &json!({"version": 1.0})).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Warp { version: live::WARP_NET, .. }), "{:?}", kind(&s, e));
    assert!(quarter(&s, e).distance(at(150.0, 122.5)) < 1e-9, "{:?}", quarter(&s, e));
    s.execute("object.envelope.resetWithWarp", &json!({"version": 0})).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Warp { version: 0, .. }), "{:?}", kind(&s, e));
    let (_, old) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "flag", "bend": 40, "version": 0}));
    assert!(matches!(kind(&s, old), EnvelopeKind::Warp { version: 0, .. }), "{:?}", kind(&s, old));
    for bad in [json!(2), json!(-1), json!(0.5), json!("1"), json!(null)] {
        for cmd in ["object.envelope.makeWithWarp", "object.envelope.resetWithWarp", "object.envelope.options"] {
            let r = s.execute(cmd, &json!({"version": bad}));
            assert!(r.as_ref().is_err_and(|e| e.to_string().contains("`version` must be 0 or 1")), "{cmd} {bad}: {r:?}");
        }
    }
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
    // The mesh follows the arc: its top corners swing out past the content's sides and up.
    let b = m.outline().bounds().unwrap();
    assert!(b.x0 < 99.0 && b.x1 > 301.0 && b.y0 < 99.0, "{b:?}");
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

/// Envelope Options change the options they name: the margins an envelope opens with from a file
/// stay, and they survive a native save.
#[test]
fn envelope_options_keep_the_margins_of_an_opened_file() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1}));
    assert_eq!(options(&s, e).0.bounds_margin, [0.0; 4]);
    let margin = [0.01, 0.02, 0.01, 0.02];
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    if let Some(NodeKind::Envelope { options, .. }) = doc.node_mut(e).map(|n| &mut n.kind) {
        options.bounds_margin = margin;
    }
    s.execute("object.envelope.options", &json!({"antiAlias": false, "fidelity": 75})).unwrap();
    assert_eq!(options(&s, e).0.bounds_margin, margin);
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(back.node(e), d.node(e));
    assert_eq!(s.execute("object.envelope.info", &json!({})).unwrap()["boundsMargin"], json!(margin));
    s.execute("object.envelope.options", &json!({"distortAppearance": false})).unwrap();
    assert_eq!(options(&s, e).0.bounds_margin, margin);
}

/// `object.envelope.info` leaves out margins that are zero.
#[test]
fn envelope_info_reports_margins_only_when_it_has_them() {
    let mut s = session();
    enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1}));
    assert!(s.execute("object.envelope.info", &json!({})).unwrap().get("boundsMargin").is_none());
}

/// Leaving Edit Contents measures the margins again when a stroke changed: a wider stroke gives
/// larger ones and no stroke gives none, after which a stroke adds none, as in a new envelope. A
/// new envelope's margins stay zero.
#[test]
fn leaving_edit_contents_measures_the_margins_again() {
    let mut s = session();
    let (a, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1}));
    let set_stroke = |s: &mut Session, weight: f64| {
        sel(s, &[e]);
        s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
        sel(s, &[a]);
        s.execute("stroke.set", &json!({"weight": weight})).unwrap();
        sel(s, &[e]);
        s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    };
    set_stroke(&mut s, 10.0);
    assert_eq!(options(&s, e).0.bounds_margin, [0.0; 4]);

    let margin = [0.05, 0.05, 0.025, 0.025];
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    if let Some(NodeKind::Envelope { options, .. }) = doc.node_mut(e).map(|n| &mut n.kind) {
        options.bounds_margin = [0.5; 4];
    }
    set_stroke(&mut s, 20.0);
    assert_eq!(options(&s, e).0.bounds_margin, margin.map(|m| m * 2.0));
    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    assert_eq!(options(&s, e).0.bounds_margin, margin.map(|m| m * 2.0));
    s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    assert_eq!(options(&s, e).0.bounds_margin, margin.map(|m| m * 2.0));
    set_stroke(&mut s, 10.0);
    assert_eq!(options(&s, e).0.bounds_margin, margin);
    // A stroke change, then a second entry, undone back to the first: leaving measures the change.
    sel(&mut s, &[e]);
    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    sel(&mut s, &[a]);
    s.execute("stroke.set", &json!({"weight": 30.0})).unwrap();
    sel(&mut s, &[e]);
    s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[e]);
    s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    assert_eq!(options(&s, e).0.bounds_margin, margin.map(|m| m * 3.0));
    set_stroke(&mut s, 0.0);
    assert_eq!(options(&s, e).0.bounds_margin, [0.0; 4]);
    set_stroke(&mut s, 10.0);
    assert_eq!(options(&s, e).0.bounds_margin, [0.0; 4]);
}

/// After an uneven scale, Edit Contents without a stroke change leaves the margins and the box the
/// content maps from, and a stroke change measures them.
#[test]
fn edit_contents_keeps_the_margins_of_a_scaled_envelope_until_a_stroke_changes() {
    let mut s = session();
    let (a, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1}));
    sel(&mut s, &[a]);
    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    s.execute("stroke.set", &json!({"weight": 10.0})).unwrap();
    sel(&mut s, &[e]);
    s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    let margin = [0.05, 0.05, 0.025, 0.025];
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    if let Some(NodeKind::Envelope { options, .. }) = doc.node_mut(e).map(|n| &mut n.kind) {
        options.bounds_margin = margin;
    }
    s.execute("object.transform", &json!({"ids": [e.0], "matrix": [2, 0, 0, 0.5, 0, 0]})).unwrap();
    let src = |s: &Session| live::EnvelopeMap::of(s.doc().unwrap().doc.node(e).unwrap()).unwrap().src();
    let box_before = src(&s);
    sel(&mut s, &[e]);
    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    assert_eq!(options(&s, e).0.bounds_margin, margin);
    assert_eq!(src(&s), box_before);
    // Undoing that step and leaving again gives the same.
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[e]);
    s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    assert_eq!(options(&s, e).0.bounds_margin, margin);
    // So does saving natively in Edit Contents, reopening and leaving.
    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(back.node(e), s.doc().unwrap().doc.node(e));
    assert!(back.node(e).unwrap().extras().envelope_edit_margin.is_some());
    s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    assert!(node(&s, e).extras().envelope_edit_margin.is_none());

    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    sel(&mut s, &[a]);
    s.execute("stroke.set", &json!({"weight": 0.0})).unwrap();
    sel(&mut s, &[e]);
    s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    assert_eq!(options(&s, e).0.bounds_margin, [0.0; 4]);
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

/// The glyph outline bounds of type `id` in document space.
fn ink_of(s: &Session, id: NodeId) -> Rect {
    match node(s, id).kind {
        NodeKind::Text(t) => t.xf.transform_rect_bbox(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).ink_bounds().unwrap()),
        k => panic!("{k:?}"),
    }
}

fn assert_box(a: Rect, b: Rect, what: &str) {
    let d = [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(d < 1e-6, "{what}: {a:?} vs {b:?}");
}

/// The box envelope `id` maps its content from.
fn src_of(s: &Session, id: NodeId) -> Rect {
    live::EnvelopeMap::of(&node(s, id)).unwrap().src()
}

/// Envelopes map point type, area type and type in a group from the glyph outline bounds, not
/// from the laid-out box (ascent to descent, or the area's frame): with a flat warp the distorted
/// type sits where the type was, and Make with Mesh lays its grid over the glyphs.
#[test]
fn envelopes_map_type_from_its_glyph_outline_bounds() {
    let mut s = session();
    let point = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Envelope", "size": 48})).unwrap());
    let area = id_of(
        &s.execute("text.create", &json!({"x": 100, "y": 300, "text": "Area type", "size": 36, "area": {"width": 300, "height": 200}})).unwrap(),
    );
    let grouped = id_of(&s.execute("text.create", &json!({"x": 450, "y": 200, "text": "Group", "size": 40})).unwrap());
    sel(&mut s, &[grouped]);
    let group = id_of(&s.execute("object.group", &json!({})).unwrap());
    for (t, target) in [(point, point), (area, area), (grouped, group)] {
        let (ink, laid) = (ink_of(&s, t), node(&s, t).geometric_bounds().unwrap());
        assert!(laid.height() - ink.height() > 1.0, "the laid-out box {laid:?} reaches past the glyphs {ink:?}");
        let (stroked, plain) = live::envelope_content_boxes(&[std::sync::Arc::new(node(&s, target))], Affine::IDENTITY).unwrap();
        assert_box(plain, ink, "geometric box");
        assert_box(stroked, ink, "box with strokes (none)");
        sel(&mut s, &[target]);
        let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
        assert_box(src_of(&s, e), ink, "warp");
        assert_box(node(&s, e).geometric_bounds().unwrap(), ink, "flat warp's bounds");
        s.execute("edit.undo", &json!({})).unwrap();
        let e = id_of(&s.execute("object.envelope.makeWithMesh", &json!({"rows": 2, "cols": 2})).unwrap());
        let EnvelopeKind::Mesh { points, .. } = kind(&s, e) else { panic!("mesh") };
        let grid = points.iter().fold(Rect::from_points(points[0], points[0]), |r, p| r.union_pt(*p));
        assert_box(grid, ink, "mesh grid");
        assert_box(src_of(&s, e), ink, "mesh");
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

/// Shrink Text to Fit lays type out by its size on the page: after the envelope is scaled, it
/// maps from the glyphs of the type laid out at its new size, not those measured before.
#[test]
fn a_scaled_envelope_of_shrunk_type_maps_from_its_new_glyphs() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 2, "area": {"width": 3, "height": 3}})).unwrap());
    s.edit("shrink", |d, _| {
        if let NodeKind::Text(tx) = &mut d.node_mut(t).unwrap().kind {
            tx.area.fit = vectorcraft_doc::AreaFit::ShrinkText { min_percent: 10.0 };
        }
        Ok(())
    })
    .unwrap();
    sel(&mut s, &[t]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
    let text = |s: &Session| match node(s, e).kind {
        NodeKind::Envelope { content, .. } => match &content[0].kind {
            NodeKind::Text(tx) => tx.clone(),
            k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    };
    let ink = |tx: &vectorcraft_doc::TextObject| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx).ink_bounds().unwrap();
    let before = text(&s);
    assert_box(src_of(&s, e), before.xf.transform_rect_bbox(ink(&before)), "before scaling");
    s.execute("object.scale", &json!({"sx": 150, "sy": 150, "origin": [100, 100]})).unwrap();
    let after = text(&s);
    let fresh = after.xf.transform_rect_bbox(ink(&after));
    let stale = after.xf.transform_rect_bbox(ink(&before));
    assert!((fresh.width() - stale.width()).abs() > 1e-3 || (fresh.height() - stale.height()).abs() > 1e-3, "{fresh:?} vs {stale:?}");
    assert_box(src_of(&s, e), fresh, "after scaling");
}

/// A rotated envelope keeps a frame of its own: shrunk type in it is measured as it is laid out on
/// the page, and its box is then taken into the frame.
#[test]
fn a_rotated_envelope_of_shrunk_type_maps_from_the_glyphs_on_the_page() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 2, "area": {"width": 3, "height": 3}})).unwrap());
    s.edit("shrink", |d, _| {
        if let NodeKind::Text(tx) = &mut d.node_mut(t).unwrap().kind {
            tx.area.fit = vectorcraft_doc::AreaFit::ShrinkText { min_percent: 10.0 };
        }
        Ok(())
    })
    .unwrap();
    sel(&mut s, &[t]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
    s.execute("object.rotate", &json!({"angle": 30, "origin": [100, 100]})).unwrap();
    s.execute("object.scale", &json!({"sx": 150, "sy": 150, "origin": [100, 100]})).unwrap();
    let (frame, tx) = match node(&s, e).kind {
        NodeKind::Envelope { frame, content, .. } => match &content[0].kind {
            NodeKind::Text(tx) => (*frame, tx.clone()),
            k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    };
    assert!(frame != Affine::IDENTITY, "a rotated envelope has a frame");
    let ink = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tx).ink_bounds().unwrap();
    assert_box(src_of(&s, e), (frame.inverse() * tx.xf).transform_rect_bbox(ink), "in the frame");
}

/// The bars of underlined type are part of what it draws, so the envelope maps from them too and
/// its bounds take them in.
#[test]
fn an_envelope_maps_underlined_type_from_its_glyphs_and_bars() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "HH", "size": 100})).unwrap());
    s.execute("text.setFormat", &json!({"ids": [t.0], "underline": true})).unwrap();
    let glyphs = ink_of(&s, t);
    sel(&mut s, &[t]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
    let src = src_of(&s, e);
    assert!(src.y1 > glyphs.y1 + 1.0, "the underline below the glyphs {glyphs:?} is in the box {src:?}");
    assert_box(node(&s, e).geometric_bounds().unwrap(), src, "flat warp's bounds");
    let drawn = vectorcraft_render::effects::expanded_live_group(Some(&s.doc().unwrap().doc), &node(&s, e)).geometric_bounds().unwrap();
    let off = [drawn.x0 - src.x0, drawn.y0 - src.y0, drawn.x1 - src.x1, drawn.y1 - src.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(off < 0.05, "the drawn art {drawn:?} fills the box {src:?}");
}

/// Undo of Make Envelope gives the type back with its laid-out box, and redo maps it from its
/// glyphs again.
#[test]
fn undo_and_redo_of_an_envelope_of_type_keep_the_glyph_mapping() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Undo", "size": 60})).unwrap());
    let (ink, laid) = (ink_of(&s, t), node(&s, t).geometric_bounds().unwrap());
    sel(&mut s, &[t]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "bulge", "bend": 30})).unwrap());
    let made = node(&s, e).geometric_bounds().unwrap();
    assert_box(src_of(&s, e), ink, "made");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(e).is_none());
    assert_box(node(&s, t).geometric_bounds().unwrap(), laid, "the type after undo");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_box(src_of(&s, e), ink, "redone");
    assert_box(node(&s, e).geometric_bounds().unwrap(), made, "redone bounds");
}

/// A saved envelope of type reopens drawn by the same rule: from the glyph outline bounds, which
/// the file does not store.
#[test]
fn a_saved_envelope_of_type_reopens_mapped_from_its_glyphs() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Saved", "size": 72})).unwrap());
    let ink = ink_of(&s, t);
    sel(&mut s, &[t]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "flag", "bend": 40})).unwrap());
    let render = |s: &Session| vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 600, 400, Affine::IDENTITY, &Default::default());
    let before = render(&s);
    let dir = std::env::temp_dir().join(format!("vectorcraft-envelope-ink-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("type.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    let mut o = Session::new();
    o.execute("document.open", &json!({"path": path})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_box(src_of(&o, e), ink, "reopened");
    let after = render(&o);
    let differ = (0..400).flat_map(|y| (0..600).map(move |x| (x, y))).filter(|&(x, y)| before.pixel(x, y) != after.pixel(x, y)).count();
    assert_eq!(differ, 0, "{differ} pixels changed on reopening");
}

/// The type of envelope `e`'s first content object.
fn content_text(s: &Session, e: NodeId) -> vectorcraft_doc::TextObject {
    match node(s, e).kind {
        NodeKind::Envelope { content, .. } => match &content[0].kind {
            NodeKind::Text(tx) => (**tx).clone(),
            k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    }
}

/// The box of `tx`'s glyph outlines with text space mapped by `m`, each outline mapped point by point.
fn outlines_in(tx: &vectorcraft_doc::TextObject, m: Affine) -> Rect {
    let path = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx).to_bezpath();
    kurbo::Shape::bounding_box(&(m * path))
}

/// Rotated type maps from the box of its rotated glyph outlines, which is smaller than the box of
/// its rotated glyph box; in a rotated envelope whose frame turns by another angle than the type,
/// it maps from the outlines taken into the frame.
#[test]
fn an_envelope_of_rotated_type_maps_from_its_rotated_outlines() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 200, "y": 300, "text": "Rotated", "size": 60})).unwrap());
    sel(&mut s, &[t]);
    s.execute("object.rotate", &json!({"angle": 40, "origin": [200, 300]})).unwrap();
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
    let tx = content_text(&s, e);
    let outlines = outlines_in(&tx, tx.xf);
    let ink = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tx).ink_bounds().unwrap();
    let glyph_box = tx.xf.transform_rect_bbox(ink);
    assert!(glyph_box.width() - outlines.width() > 1.0 || glyph_box.height() - outlines.height() > 1.0, "{glyph_box:?} vs {outlines:?}");
    assert_box(src_of(&s, e), outlines, "rotated type");
    s.execute("object.rotate", &json!({"angle": 15, "origin": [200, 300]})).unwrap();
    let (frame, tx) = match node(&s, e).kind {
        NodeKind::Envelope { frame, .. } => (*frame, content_text(&s, e)),
        k => panic!("{k:?}"),
    };
    assert!(frame != Affine::IDENTITY, "a rotated envelope has a frame");
    let in_frame = frame.inverse() * tx.xf;
    assert!(in_frame.as_coeffs()[1].abs() > 0.1, "the type is turned against the frame: {in_frame:?}");
    assert_box(src_of(&s, e), outlines_in(&tx, in_frame), "in the frame");
}

/// Type in perspective maps from the box of its outlines as its projection draws them, which is
/// smaller than the projected box of its glyphs. Where the projection can't map a point (past its
/// horizon), the point counts where it is, as the renderer draws it.
#[test]
fn an_envelope_of_type_in_perspective_maps_from_its_projected_outlines() {
    use vectorcraft_geom::Homography;
    // Every point of the type is past this one's horizon.
    let horizon = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]]);
    for (h, crosses) in [(Homography([[1.0, 0.3, 0.0], [0.1, 1.0, 0.0], [0.0015, 0.0008, 1.0]]), false), (horizon, true)] {
        let mut s = session();
        let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Perspective", "size": 60})).unwrap());
        s.edit("perspective", |d, _| {
            let n = d.node_mut(t).unwrap();
            n.extras_mut().perspective = Some(Box::new(vectorcraft_doc::PerspectiveAttachment {
                projection: Some(h.to_array()),
                ..vectorcraft_doc::PerspectiveAttachment::new("left", 0.0)
            }));
            Ok(())
        })
        .unwrap();
        assert!(node(&s, t).projection().is_some());
        let NodeKind::Text(tx) = node(&s, t).kind else { panic!("type") };
        let path = tx.xf * vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tx).to_bezpath();
        let (mut drawn, mut unmapped): (Option<Rect>, usize) = (None, 0);
        kurbo::flatten(&path, 0.001, |el| {
            if let kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) = el {
                let q = h.apply(p).unwrap_or_else(|| {
                    unmapped += 1;
                    p
                });
                drawn = Some(drawn.map_or(Rect::from_points(q, q), |r| r.union_pt(q)));
            }
        });
        let drawn = drawn.unwrap();
        assert_eq!(unmapped > 0, crosses, "{unmapped} points past the horizon");
        if !crosses {
            let ink = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tx).ink_bounds().unwrap();
            let boxed = h.map_rect_bbox(tx.xf.transform_rect_bbox(ink)).unwrap();
            assert!(boxed.width() - drawn.width() > 1.0 || boxed.height() - drawn.height() > 1.0, "{boxed:?} vs {drawn:?}");
        }
        sel(&mut s, &[t]);
        let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
        let src = src_of(&s, e);
        let off = [src.x0 - drawn.x0, src.y0 - drawn.y0, src.x1 - drawn.x1, src.y1 - drawn.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(off < 0.05, "{src:?} vs {drawn:?}");
    }
}

/// Type that draws nothing (only spaces) keeps its laid-out box.
#[test]
fn an_envelope_of_type_that_draws_nothing_maps_from_its_laid_out_box() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "   ", "size": 48})).unwrap());
    let laid = node(&s, t).geometric_bounds().unwrap();
    assert_box(live::envelope_content_bounds(&[std::sync::Arc::new(node(&s, t))]).unwrap(), laid, "spaces");
    sel(&mut s, &[t]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
    assert_box(src_of(&s, e), laid, "warp");
}

/// Type that draws nothing adds no box next to other content: Make with Warp and Make with Mesh
/// around it and a rectangle, alone or in a group, map from the rectangle.
#[test]
fn type_that_draws_nothing_adds_no_box_next_to_other_content() {
    for (cmd, params) in
        [("object.envelope.makeWithWarp", json!({"style": "arc", "bend": 0})), ("object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1}))]
    {
        for grouped in [false, true] {
            let mut s = session();
            let a = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
            let want = node(&s, a).geometric_bounds().unwrap();
            let t = id_of(&s.execute("text.create", &json!({"x": 400, "y": 450, "text": "   ", "size": 96})).unwrap());
            let laid = node(&s, t).geometric_bounds().unwrap();
            assert!(laid.x1 > want.x1 && laid.y1 > want.y1, "the laid-out box {laid:?} reaches past {want:?}");
            sel(&mut s, &[a, t]);
            if grouped {
                s.execute("object.group", &json!({})).unwrap();
            }
            let layer = s.doc().unwrap().doc.layers[0].clone();
            assert_box(live::envelope_content_bounds(&[layer]).unwrap(), want, "content bounds");
            let e = id_of(&s.execute(cmd, &params).unwrap());
            assert_box(src_of(&s, e), want, &format!("{cmd}, grouped: {grouped}"));
        }
    }
}

/// Deleting every character of enveloped type in Edit Contents leaves the envelope mapping from
/// the rectangle beside it; with nothing beside it, from the type's laid-out box.
#[test]
fn edit_contents_that_empties_type_maps_from_the_other_content() {
    for with_rect in [true, false] {
        let mut s = session();
        let a = with_rect.then(|| rect(&mut s, 100.0, 100.0, 200.0, 100.0));
        let rect_box = a.map(|a| node(&s, a).geometric_bounds().unwrap());
        let t = id_of(&s.execute("text.create", &json!({"x": 400, "y": 450, "text": "Type", "size": 96})).unwrap());
        sel(&mut s, &a.into_iter().chain([t]).collect::<Vec<_>>());
        let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
        sel(&mut s, &[e]);
        s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
        s.execute("text.setText", &json!({"ids": [t.0], "text": " "})).unwrap();
        sel(&mut s, &[e]);
        s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
        let NodeKind::Envelope { content, .. } = node(&s, e).kind else { panic!("envelope") };
        let blank = content.iter().find(|c| c.id == t).expect("the type stays");
        assert!(matches!(&blank.kind, NodeKind::Text(tx) if tx.plain_text().trim().is_empty()), "the type was emptied");
        let want = rect_box.unwrap_or_else(|| blank.geometric_bounds().unwrap());
        assert_box(src_of(&s, e), want, &format!("with a rectangle: {with_rect}"));
    }
}

/// The strikethrough bar runs to the last glyph's advance, past its outline: the envelope maps
/// from it too.
#[test]
fn an_envelope_maps_struck_through_type_from_its_glyphs_and_bars() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "HH", "size": 100})).unwrap());
    let plain = ink_of(&s, t);
    s.execute("text.setFormat", &json!({"ids": [t.0], "strikethrough": true})).unwrap();
    let NodeKind::Text(tx) = node(&s, t).kind else { panic!("type") };
    let db = vectorcraft_text::FontDb::global();
    let laid = vectorcraft_text::layout(db, &tx);
    let bars = vectorcraft_text::decorations(&laid, db, &tx);
    assert!(!bars.is_empty(), "struck-through type has a bar");
    let bar = tx.xf.transform_rect_bbox(kurbo::Shape::bounding_box(&bars[0].1));
    assert!(bar.x1 > plain.x1 + 1.0, "the bar {bar:?} reaches past the glyphs {plain:?}");
    sel(&mut s, &[t]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
    assert_box(src_of(&s, e), plain.union(bar), "glyphs and bar");
}

/// In a layer, the visible type is measured by its glyphs and hidden type isn't measured; in a
/// clip group whose clipping path is type, the envelope maps from that type's glyphs.
#[test]
fn type_in_layers_and_clip_groups_is_measured_by_its_glyphs() {
    let mut s = session();
    let shown = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Shown", "size": 48})).unwrap());
    let hidden = id_of(&s.execute("text.create", &json!({"x": 100, "y": 500, "text": "Hidden", "size": 48})).unwrap());
    sel(&mut s, &[hidden]);
    s.execute("object.hide", &json!({})).unwrap();
    let ink = ink_of(&s, shown);
    let layer = s.doc().unwrap().doc.layers[0].clone();
    assert_box(live::envelope_content_bounds(&[layer]).unwrap(), ink, "layer");

    let mut s = session();
    let back = rect(&mut s, 50.0, 100.0, 500.0, 200.0);
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Clip", "size": 72})).unwrap());
    let ink = ink_of(&s, t);
    sel(&mut s, &[back, t]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    assert!(matches!(node(&s, g).kind, NodeKind::Group { clip: true, .. }));
    let (stroked, plain) = live::envelope_content_boxes(&[std::sync::Arc::new(node(&s, g))], Affine::IDENTITY).unwrap();
    assert_box(plain, ink, "clip group");
    assert_box(stroked, ink, "clip group with strokes (none)");
    sel(&mut s, &[g]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
    assert_box(src_of(&s, e), ink, "warp of the clip group");
}

/// Stroked type: the box with strokes reaches past the glyphs as far as the type's strokes reach
/// past its laid-out box. In perspective, the glyphs are the projected outlines and the reach is
/// that of the strokes past the projected laid-out box.
#[test]
fn stroked_type_maps_from_its_glyphs_grown_by_its_strokes() {
    let h = vectorcraft_geom::Homography([[1.0, 0.3, 0.0], [0.1, 1.0, 0.0], [0.0015, 0.0008, 1.0]]);
    for perspective in [false, true] {
        let mut s = session();
        let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Stroke", "size": 60})).unwrap());
        sel(&mut s, &[t]);
        s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
        s.execute("stroke.set", &json!({"weight": 6})).unwrap();
        if perspective {
            s.edit("perspective", |d, _| {
                d.node_mut(t).unwrap().extras_mut().perspective = Some(Box::new(vectorcraft_doc::PerspectiveAttachment {
                    projection: Some(h.to_array()),
                    ..vectorcraft_doc::PerspectiveAttachment::new("left", 0.0)
                }));
                Ok(())
            })
            .unwrap();
        }
        let n = node(&s, t);
        assert_eq!(n.projection().is_some(), perspective);
        let (v, g) = (n.visual_bounds().unwrap(), n.geometric_bounds().unwrap());
        assert!(v.x0 < g.x0 - 1.0, "the strokes {v:?} reach past the laid-out box {g:?}");
        let NodeKind::Text(tx) = &n.kind else { panic!("type") };
        // The glyph outlines as drawn.
        let ink = if perspective {
            let path = tx.xf * vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx).to_bezpath();
            let mut drawn: Option<Rect> = None;
            kurbo::flatten(&path, 0.001, |el| {
                if let kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) = el {
                    let q = h.apply(p).unwrap();
                    drawn = Some(drawn.map_or(Rect::from_points(q, q), |r| r.union_pt(q)));
                }
            });
            drawn.unwrap()
        } else {
            ink_of(&s, t)
        };
        let (stroked, plain) = live::envelope_content_boxes(&[std::sync::Arc::new(n.clone())], Affine::IDENTITY).unwrap();
        let near = |a: Rect, b: Rect, what: &str| {
            let d = [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
            assert!(d < 0.05, "{what} (perspective {perspective}): {a:?} vs {b:?}");
        };
        near(plain, ink, "geometric box");
        near(stroked, Rect::new(ink.x0 - (g.x0 - v.x0), ink.y0 - (g.y0 - v.y0), ink.x1 + (v.x1 - g.x1), ink.y1 + (v.y1 - g.y1)), "with strokes");
    }
}

/// Make with Top Object maps type from its glyphs onto the path's outline.
#[test]
fn a_top_object_envelope_of_type_maps_from_its_glyphs() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Top", "size": 72})).unwrap());
    let ink = ink_of(&s, t);
    let top = rect(&mut s, 80.0, 100.0, 300.0, 150.0);
    sel(&mut s, &[t, top]);
    let e = id_of(&s.execute("object.envelope.makeWithTopObject", &json!({})).unwrap());
    assert!(matches!(kind(&s, e), EnvelopeKind::TopObject { .. }));
    assert_box(src_of(&s, e), ink, "top object");
}

/// Type in a symbol instance is measured as before: the envelope maps from the instance's own
/// bounds, not from the type's glyphs.
#[test]
fn an_envelope_of_a_symbol_instance_of_type_maps_from_the_instance_s_bounds() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Symbol", "size": 60})).unwrap());
    let ink = ink_of(&s, t);
    sel(&mut s, &[t]);
    s.execute("symbol.new", &json!({"name": "Type"})).unwrap();
    let inst = s.doc().unwrap().selection.objects[0];
    assert!(matches!(node(&s, inst).kind, NodeKind::SymbolInstance { .. }));
    let own = node(&s, inst).geometric_bounds().unwrap();
    assert!((own.height() - ink.height()).abs() > 1.0, "{own:?} vs {ink:?}");
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
    assert_box(src_of(&s, e), own, "symbol instance");
}

/// Type inside a blend or a repeat is measured as before: the envelope maps from the object's own
/// bounds, not from the type's glyphs.
#[test]
fn an_envelope_of_a_blend_or_repeat_of_type_maps_from_its_own_bounds() {
    for repeat in [false, true] {
        let mut s = session();
        let a = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Blend", "size": 60})).unwrap());
        let mut parts = vec![a];
        if !repeat {
            parts.push(id_of(&s.execute("text.create", &json!({"x": 300, "y": 400, "text": "Blend", "size": 60})).unwrap()));
        }
        let ink = parts.iter().map(|t| ink_of(&s, *t)).reduce(|a, b| a.union(b)).unwrap();
        sel(&mut s, &parts);
        let made = if repeat { "object.repeat.grid" } else { "object.blend.make" };
        let holder = id_of(&s.execute(made, &json!({})).unwrap());
        let own = node(&s, holder).geometric_bounds().unwrap();
        let measured = live::envelope_content_bounds(&[std::sync::Arc::new(node(&s, holder))]).unwrap();
        assert_box(measured, own, made);
        assert!((own.height() - ink.height()).abs() > 1.0 || (own.width() - ink.width()).abs() > 1.0, "{made}: {own:?} vs {ink:?}");
        sel(&mut s, &[holder]);
        let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
        assert_box(src_of(&s, e), own, made);
    }
}

/// An inline graphic counts by its box: the envelope maps from the glyph outlines and the box of
/// the graphic, which reaches past the glyphs.
#[test]
fn an_envelope_maps_type_with_an_inline_graphic_from_its_glyphs_and_the_graphic() {
    let mut s = session();
    s.execute("shape.ellipse", &json!({"x": 600, "y": 500, "width": 40, "height": 40})).unwrap();
    s.execute("symbol.new", &json!({"name": "Dot"})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "xx", "size": 40})).unwrap());
    s.execute("text.insertInline", &json!({"id": t.0, "at": 1, "symbol": "Dot"})).unwrap();
    let NodeKind::Text(tx) = node(&s, t).kind else { panic!("type") };
    let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tx);
    let [inline] = laid.inlines.as_slice() else { panic!("one inline graphic: {}", laid.inlines.len()) };
    let graphic = tx.xf.transform_rect_bbox(inline.bounds);
    let glyphs = outlines_in(&tx, tx.xf);
    assert!(graphic.y0 < glyphs.y0 - 1.0, "the graphic {graphic:?} reaches above the glyphs {glyphs:?}");
    sel(&mut s, &[t]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 0})).unwrap());
    assert_box(src_of(&s, e), glyphs.union(graphic), "glyphs and graphic");
}

/// Reset with Mesh without Maintain Shape lays its flat grid over the glyphs.
#[test]
fn reset_with_mesh_lays_a_flat_grid_over_the_glyphs() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "Reset", "size": 72})).unwrap());
    let ink = ink_of(&s, t);
    sel(&mut s, &[t]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "bulge", "bend": 40})).unwrap());
    s.execute("object.envelope.resetWithMesh", &json!({"rows": 2, "cols": 3, "maintainShape": false})).unwrap();
    let EnvelopeKind::Mesh { points, .. } = kind(&s, e) else { panic!("mesh") };
    let grid = points.iter().fold(Rect::from_points(points[0], points[0]), |r, p| r.union_pt(*p));
    assert_box(grid, ink, "flat grid");
}
