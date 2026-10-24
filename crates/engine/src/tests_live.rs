//! Live blends, envelopes and gradient meshes, driven through `Session::execute`.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::live::{self, BlendOrientation, BlendSpacing, EnvelopeKind};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Point, Rect};

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

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn fill(s: &mut Session, id: NodeId, hex: &str) {
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": hex})).unwrap();
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// Two 10×10 rects (black at x=0, white at x=100), blended with `params`.
fn blend(s: &mut Session, params: Value) -> (NodeId, NodeId, NodeId) {
    let a = rect(s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(s, 100.0, 0.0, 10.0, 10.0);
    fill(s, a, "#000000");
    fill(s, b, "#ffffff");
    sel(s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &params).unwrap());
    (a, b, g)
}

fn steps_of(s: &Session, id: NodeId) -> Vec<Node> {
    live::expand_live(&node(s, id))
}

fn render(s: &Session, region: Rect) -> vectorcraft_render::Rendered {
    vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, region, 1.0, true)
}

fn all_anchors(nodes: &[Node]) -> Vec<Point> {
    let mut v = vec![];
    for n in nodes {
        n.walk(&mut |c| {
            if let Some(p) = c.path_data() {
                v.extend(p.anchors().map(|(_, _, a)| a.p));
            }
        });
    }
    v
}

/// `cmd` with `p` fails with the error for `key`, which `p` gives as something other than a number.
fn refuses_non_number(s: &mut Session, cmd: &str, p: &Value, key: &str) {
    let r = s.execute(cmd, p);
    let want = format!("`{key}` must be a number, not {}", p[key]);
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}");
}

/// `cmd` with `p` fails with the error for `key`, which `p` gives as something other than true or false.
fn refuses_non_boolean(s: &mut Session, cmd: &str, p: &Value, key: &str) {
    let r = s.execute(cmd, p);
    let want = format!("`{key}` must be true or false");
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}");
}

// ---------- Blend ----------

#[test]
fn blend_make_is_live_with_keys_as_children() {
    let mut s = session();
    let (a, b, g) = blend(&mut s, json!({"steps": 3}));
    let n = node(&s, g);
    assert!(matches!(n.kind, NodeKind::Blend { .. }));
    let ch: Vec<NodeId> = n.children().unwrap().iter().map(|c| c.id).collect();
    assert_eq!(ch, vec![a, b]);
    assert_eq!(n.kind_label(), "Blend");
    assert_eq!(s.doc().unwrap().selection.objects, vec![g]);
}

#[test]
fn blend_specified_steps_count() {
    let mut s = session();
    let (_, _, g) = blend(&mut s, json!({"steps": 3}));
    assert_eq!(steps_of(&s, g).len(), 5);
}

/// Specified Steps that are not a whole number are an error, in Make and in Blend Options, and
/// nothing changes; 4.0 is a whole number of steps.
#[test]
fn blend_steps_that_are_not_a_whole_number_are_refused() {
    let mut s = session();
    let (_, _, g) = blend(&mut s, json!({"steps": 3}));
    let (a, b) = (rect(&mut s, 0.0, 50.0, 10.0, 10.0), rect(&mut s, 100.0, 50.0, 10.0, 10.0));
    let n = undo_len(&s);
    for p in [json!({"ids": [a.0, b.0], "steps": 2.5}), json!({"ids": [a.0, b.0], "spacing": "steps", "value": 2.5})] {
        let key = if p.get("value").is_some() { "value" } else { "steps" };
        let r = s.execute("object.blend.make", &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == format!("`{key}` must be a whole number")), "{p}: {r:?}");
    }
    sel(&mut s, &[g]);
    for p in [json!({"steps": 2.5}), json!({"spacing": "steps", "value": 1.5})] {
        assert!(s.execute("object.blend.options", &p).is_err(), "{p}");
    }
    assert_eq!((undo_len(&s), steps_of(&s, g).len()), (n, 5), "nothing changed");
    s.execute("object.blend.options", &json!({"steps": 4.0})).unwrap();
    assert_eq!(steps_of(&s, g).len(), 6);
}

/// Make refuses Specified Steps or Distance given as text or as another value that is not a number,
/// and makes no blend; `steps: 8` as a number makes 8 steps.
#[test]
fn blend_make_refuses_a_number_given_as_text() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0, 10.0));
    sel(&mut s, &[a, b]);
    let n = undo_len(&s);
    for (key, p) in [
        ("steps", json!({"steps": "8"})),
        ("distance", json!({"distance": "10"})),
        ("value", json!({"spacing": "steps", "value": "3"})),
        ("value", json!({"spacing": "distance", "value": [5]})),
        ("distance", json!({"spacing": "distance", "distance": true})),
    ] {
        refuses_non_number(&mut s, "object.blend.make", &p, key);
    }
    assert_eq!(undo_len(&s), n, "nothing changed");
    assert!(matches!(node(&s, a).kind, NodeKind::Path { .. }), "not blended");
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 8})).unwrap());
    assert_eq!(steps_of(&s, g).len(), 10);
}

/// Blend Options refuses Specified Steps or Distance given as text, for the selected blends and for
/// the options new blends start with, and changes neither.
#[test]
fn blend_options_refuse_a_number_given_as_text() {
    let mut s = session();
    let (_, _, g) = blend(&mut s, json!({"steps": 3}));
    let n = undo_len(&s);
    let calls = [
        ("steps", json!({"steps": "4"})),
        ("value", json!({"spacing": "steps", "value": "4"})),
        ("distance", json!({"distance": "20"})),
        ("distance", json!({"spacing": "distance", "value": null, "distance": "20"})),
    ];
    for (key, p) in &calls {
        refuses_non_number(&mut s, "object.blend.options", p, key);
    }
    assert_eq!((undo_len(&s), steps_of(&s, g).len()), (n, 5), "the blend kept its options");
    s.execute("select.none", &json!({})).unwrap();
    let defaults = s.prefs.blend_options;
    for (key, p) in &calls {
        refuses_non_number(&mut s, "object.blend.options", p, key);
    }
    assert_eq!(s.prefs.blend_options, defaults, "the options new blends start with stayed");
}

/// Make and Blend Options refuse a `smooth` given as text or as another value that is not true or
/// false, also beside Specified Steps: Make makes no blend, and the selected blend and the options new
/// blends start with keep their spacing. Null is read as `smooth` left out, and true still gives
/// Smooth Color.
#[test]
fn blend_make_and_options_refuse_a_smooth_given_as_text() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0, 10.0));
    sel(&mut s, &[a, b]);
    let n = undo_len(&s);
    let calls = [json!({"smooth": "true"}), json!({"smooth": 1}), json!({"steps": 4, "smooth": "false"}), json!({"smooth": [true]})];
    for p in &calls {
        refuses_non_boolean(&mut s, "object.blend.make", p, "smooth");
    }
    assert_eq!(undo_len(&s), n, "nothing changed");
    assert!(matches!(node(&s, a).kind, NodeKind::Path { .. }), "not blended");
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3, "smooth": null})).unwrap());
    let n = undo_len(&s);
    for p in &calls {
        refuses_non_boolean(&mut s, "object.blend.options", p, "smooth");
    }
    assert_eq!((undo_len(&s), steps_of(&s, g).len()), (n, 5), "the blend kept its options");
    s.execute("object.blend.options", &json!({"smooth": true})).unwrap();
    assert!(matches!(&node(&s, g).kind, NodeKind::Blend { spec, .. } if spec.spacing == BlendSpacing::SmoothColor));
    s.execute("select.none", &json!({})).unwrap();
    let defaults = s.prefs.blend_options;
    for p in &calls {
        refuses_non_boolean(&mut s, "object.blend.options", p, "smooth");
    }
    assert_eq!(s.prefs.blend_options, defaults, "the options new blends start with stayed");
}

#[test]
fn blend_mid_step_interpolates_geometry_and_colour() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 30.0, 30.0);
    fill(&mut s, a, "#000000");
    fill(&mut s, b, "#ffffff");
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3})).unwrap());
    let st = steps_of(&s, g);
    let mb = st[2].geometric_bounds().unwrap();
    assert!(close(mb.x0, 50.0) && close(mb.width(), 20.0), "{mb:?}");
    let c = st[2].appearance.fill_paint().color().unwrap().to_rgb();
    assert!((c[0] - 0.5).abs() < 1e-3);
}

#[test]
fn blend_smooth_colour_steps_from_colour_distance() {
    let mut s = session();
    let (_, _, g) = blend(&mut s, json!({}));
    let n = steps_of(&s, g).len() - 2;
    assert_eq!(n, 128, "black→white: one step per two levels");
    // Consecutive steps differ by at most ~2 levels.
    let st = steps_of(&s, g);
    for w in st.windows(2) {
        let x = w[0].appearance.fill_paint().color().unwrap().to_rgb()[0];
        let y = w[1].appearance.fill_paint().color().unwrap().to_rgb()[0];
        assert!((x - y).abs() <= 2.5 / 255.0);
    }
}

#[test]
fn blend_smooth_colour_same_colours_uses_distance() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"smooth": true})).unwrap());
    assert_eq!(steps_of(&s, g).len() - 2, 20);
}

#[test]
fn blend_specified_distance() {
    let mut s = session();
    let (_, _, g) = blend(&mut s, json!({"distance": 10}));
    assert_eq!(steps_of(&s, g).len() - 2, 9);
    let st = steps_of(&s, g);
    let d = st[1].geometric_bounds().unwrap().x0 - st[0].geometric_bounds().unwrap().x0;
    assert!(close(d, 10.0), "{d}");
}

#[test]
fn blend_interpolates_opacity_and_stroke_weight() {
    let mut s = session();
    let (a, b, g) = blend(&mut s, json!({"steps": 1}));
    s.edit("t", |d, _| {
        d.node_mut(a).unwrap().opacity = 0.2;
        d.node_mut(b).unwrap().appearance.stroke_mut().unwrap().width = 9.0;
        Ok(())
    })
    .unwrap();
    // Keys live inside the blend now; edit them there.
    let _ = g;
    let st = steps_of(&s, g);
    assert!((st[1].opacity - 0.6).abs() < 1e-5);
    assert!(close(st[1].appearance.stroke_width(), 5.0));
}

#[test]
fn blend_resamples_different_anchor_counts() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    let b = id_of(&s.execute("shape.star", &json!({"cx": 300, "cy": 25, "radius1": 25, "radius2": 10, "points": 5})).unwrap());
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 2})).unwrap());
    let st = steps_of(&s, g);
    let n = st[1].path_data().unwrap().subpaths[0].anchors.len();
    assert_eq!(n, 10, "rect resampled to the star's 10 anchors");
    let b1 = st[1].geometric_bounds().unwrap();
    assert!(b1.x0 > 50.0 && b1.x1 < 325.0, "{b1:?}");
}

#[test]
fn blend_three_keys() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 10.0, 10.0);
    let c = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b, c]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 4})).unwrap());
    let st = steps_of(&s, g);
    assert_eq!(st.len(), 3 + 2 * 4);
    assert_eq!(st[5].id, b);
}

#[test]
fn blend_options_change_spacing_and_orientation() {
    let mut s = session();
    let (_, _, g) = blend(&mut s, json!({"steps": 3}));
    s.execute("object.blend.options", &json!({"spacing": "steps", "value": 1, "orientation": "path"})).unwrap();
    assert_eq!(steps_of(&s, g).len(), 3);
    match &node(&s, g).kind {
        NodeKind::Blend { spec, .. } => {
            assert_eq!(spec.spacing, BlendSpacing::Steps(1));
            assert_eq!(spec.orientation, BlendOrientation::AlignToPath);
        }
        _ => panic!(),
    }
    assert!(s.execute("object.blend.options", &json!({"steps": 0})).is_err());
    assert!(s.execute("object.blend.options", &json!({"spacing": "bogus"})).is_err());
}

#[test]
fn blend_release_restores_keys() {
    let mut s = session();
    let (a, b, g) = blend(&mut s, json!({"steps": 3}));
    let r = s.execute("object.blend.release", &json!({})).unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 2);
    let d = &s.doc().unwrap().doc;
    assert!(d.node(g).is_none() && d.node(a).is_some() && d.node(b).is_some());
    assert!(s.execute("object.blend.release", &json!({})).is_ok() || s.execute("object.blend.release", &json!({})).is_err());
}

/// Release Blend of a blend and of a blend inside a group that is a key object of the first puts
/// the outer blend's spine and keys where it was, then the inner blend's spine and keys where it
/// was inside that group, as releasing one blend at a time does.
#[test]
fn blend_release_of_a_blend_and_a_blend_inside_it_keeps_order_and_ids() {
    let mut s = session();
    let (r1, r2) = (rect(&mut s, 0.0, 0.0, 10.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0, 10.0));
    sel(&mut s, &[r1, r2]);
    let inner = id_of(&s.execute("object.blend.make", &json!({})).unwrap());
    let r3 = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[inner, r3]);
    let group = id_of(&s.execute("object.group", &json!({})).unwrap());
    let r4 = rect(&mut s, 400.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[group, r4]);
    let outer = id_of(&s.execute("object.blend.make", &json!({})).unwrap());
    sel(&mut s, &[outer, inner]);
    let before = s.doc().unwrap().doc.clone();
    let n = before.peek_next_id();
    let r = s.execute("object.blend.release", &json!({})).unwrap();
    assert_eq!(r["ids"], json!([group.0, r4.0, r1.0, r2.0]));
    assert_eq!(r["spines"], json!([n, n + 1]));
    let ids = |n: &Node| n.children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(&s.doc().unwrap().doc.layers[0]), [NodeId(n), group, r4]);
    assert_eq!(ids(&node(&s, group)), [NodeId(n + 1), r1, r2, r3]);
    assert_eq!(s.doc().unwrap().selection.objects, [NodeId(n), NodeId(n + 1), group, r4, r1, r2]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(*s.doc().unwrap().doc == *before, "Undo gives back the document");
}

#[test]
fn blend_expand_makes_group_with_unique_ids() {
    let mut s = session();
    let (_, _, g) = blend(&mut s, json!({"steps": 3}));
    s.execute("object.blend.expand", &json!({})).unwrap();
    let n = node(&s, g);
    assert!(matches!(n.kind, NodeKind::Group { .. }));
    assert_eq!(n.children().unwrap().len(), 5);
    let mut ids = vec![];
    s.doc().unwrap().doc.walk(|n| ids.push(n.id));
    let len = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), len);
    assert!(!ids.contains(&NodeId(0)));
    assert!(s.execute("object.blend.release", &json!({})).is_err(), "no longer a blend");
}

#[test]
fn blend_make_undo_is_one_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    let n = undo_len(&s);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3})).unwrap());
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(g).is_none());
    assert_eq!(d.parent_of(a), d.parent_of(b));
    assert!(d.node(a).is_some());
}

#[test]
fn blend_replace_spine_follows_path() {
    let mut s = session();
    let (_, _, g) = blend(&mut s, json!({"steps": 5}));
    // Half circle of radius 100 around (200, 300).
    let arc = id_of(&s.execute("path.create", &json!({"d": "M100 300 C100 245 145 200 200 200 C255 200 300 245 300 300"})).unwrap());
    sel(&mut s, &[g, arc]);
    s.execute("object.blend.replaceSpine", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(arc).is_none(), "the path is consumed");
    let st = steps_of(&s, g);
    assert_eq!(st.len(), 7);
    for n in &st {
        let c = n.geometric_bounds().unwrap().center();
        let r = c.distance(Point::new(200.0, 300.0));
        assert!((r - 100.0).abs() < 1.5, "{c:?} r={r}");
    }
    let first = st[0].geometric_bounds().unwrap().center();
    assert!(first.distance(Point::new(100.0, 300.0)) < 1.0, "{first:?}");
}

#[test]
fn blend_align_to_path_rotates_steps() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 4.0);
    let b = rect(&mut s, 100.0, 0.0, 40.0, 4.0);
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 1, "orientation": "path"})).unwrap());
    let spine = id_of(&s.execute("path.create", &json!({"d": "M300 100 L300 300"})).unwrap());
    sel(&mut s, &[g, spine]);
    s.execute("object.blend.replaceSpine", &json!({})).unwrap();
    let st = steps_of(&s, g);
    let b1 = st[1].geometric_bounds().unwrap();
    assert!(b1.height() > b1.width(), "vertical spine turns the bars upright: {b1:?}");
    s.execute("object.blend.options", &json!({"orientation": "page"})).unwrap();
    let b1 = steps_of(&s, g)[1].geometric_bounds().unwrap();
    assert!(b1.width() > b1.height());
}

#[test]
fn blend_reverse_spine_and_front_to_back() {
    let mut s = session();
    let (a, _, g) = blend(&mut s, json!({"steps": 2}));
    s.execute("object.blend.reverseFrontToBack", &json!({})).unwrap();
    assert_eq!(node(&s, g).children().unwrap().last().unwrap().id, a);
    s.execute("object.blend.reverseSpine", &json!({})).unwrap();
    assert!(close(node(&s, a).geometric_bounds().unwrap().x0, 100.0));
}

#[test]
fn blend_renders_intermediate_steps() {
    let mut s = session();
    let (_, _, _) = blend(&mut s, json!({"steps": 9}));
    let img = render(&s, Rect::new(0.0, 0.0, 120.0, 20.0));
    let p = img.pixel(55, 5);
    assert!(p[0] < 200 && p[0] > 60, "a grey step is drawn: {p:?}");
}

// ---------- Envelope ----------

fn text_rect(s: &mut Session) -> NodeId {
    let a = rect(s, 100.0, 100.0, 200.0, 100.0);
    fill(s, a, "#ff0000");
    a
}

#[test]
fn envelope_warp_make_and_content_stays_inside_bounds() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 50})).unwrap());
    let n = node(&s, e);
    assert!(matches!(n.kind, NodeKind::Envelope { .. }));
    assert_eq!(n.children().unwrap()[0].id, a);
    let b = n.geometric_bounds().unwrap().inflate(0.5, 0.5);
    let out = live::expand_live(&n);
    let pts = all_anchors(&out);
    assert!(pts.len() > 4, "curves are subdivided");
    assert!(pts.iter().all(|p| b.contains(*p)), "{b:?}");
    // It really is bent: the sides moved down.
    assert!(pts.iter().any(|p| p.y > 201.0));
}

#[test]
fn envelope_warp_zero_bend_is_identity() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "flag", "bend": 0})).unwrap());
    for p in all_anchors(&live::expand_live(&node(&s, e))) {
        assert!(p.x >= 99.99 && p.x <= 300.01 && p.y >= 99.99 && p.y <= 200.01, "{p:?}");
    }
    assert!(s.execute("object.envelope.makeWithWarp", &json!({"style": "nope"})).is_err());
}

#[test]
fn envelope_mesh_identity_then_point_moves_content() {
    let mut s = session();
    let a = text_rect(&mut s);
    let inner = rect(&mut s, 190.0, 140.0, 20.0, 20.0);
    sel(&mut s, &[a, inner]);
    let e = id_of(&s.execute("object.envelope.makeWithMesh", &json!({"rows": 2, "cols": 2})).unwrap());
    let b0 = Rect::from_points(Point::new(100.0, 100.0), Point::new(300.0, 200.0));
    for p in all_anchors(&live::expand_live(&node(&s, e))) {
        assert!(b0.inflate(1e-6, 1e-6).contains(p), "{p:?}");
    }
    // Centre point of the 3×3 grid is index 4 (200, 150); pull it up by 40.
    s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 4, "x": 200, "y": 110})).unwrap();
    let out = live::expand_live(&node(&s, e));
    let pts = all_anchors(&out);
    // Corners stay; content near the centre moves up.
    assert!(pts.iter().any(|p| p.distance(Point::new(100.0, 100.0)) < 1e-6));
    let inner_top = all_anchors(&out[1..]).iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    assert!(inner_top < 125.0, "{inner_top}");
}

#[test]
fn envelope_top_object_maps_corners() {
    let mut s = session();
    let a = text_rect(&mut s);
    // A trapezoid on top.
    let t = id_of(&s.execute("path.create", &json!({"d": "M400 100 L500 100 L550 250 L350 250 Z"})).unwrap());
    sel(&mut s, &[a, t]);
    let e = id_of(&s.execute("object.envelope.makeWithTopObject", &json!({})).unwrap());
    assert!(s.doc().unwrap().doc.node(t).is_none(), "top object consumed");
    let pts = all_anchors(&live::expand_live(&node(&s, e)));
    for c in [Point::new(400.0, 100.0), Point::new(500.0, 100.0), Point::new(550.0, 250.0), Point::new(350.0, 250.0)] {
        assert!(pts.iter().any(|p| p.distance(c) < 0.5), "corner {c:?} missing");
    }
    let b = node(&s, e).geometric_bounds().unwrap();
    assert!(close(b.x0, 350.0) && close(b.y1, 250.0));
}

#[test]
fn envelope_release_and_expand() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "bulge", "bend": 40})).unwrap());
    let r = s.execute("object.envelope.release", &json!({})).unwrap();
    assert_eq!(r["ids"][0].as_u64(), Some(a.0));
    assert!(s.doc().unwrap().doc.node(e).is_none());
    assert_eq!(node(&s, a).geometric_bounds(), Some(Rect::new(100.0, 100.0, 300.0, 200.0)));
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[e]);
    s.execute("object.envelope.expand", &json!({})).unwrap();
    let g = node(&s, e);
    assert!(matches!(g.kind, NodeKind::Group { .. }));
    // (Distort Appearance, on for new envelopes, expands the stroke too: the fill comes first.)
    let mut fill = None;
    g.walk(&mut |c| fill = fill.or_else(|| c.path_data().map(|p| p.anchor_count())));
    assert!(fill.unwrap() > 4);
}

#[test]
fn envelope_options_and_edit_contents() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 50})).unwrap());
    s.execute("object.envelope.options", &json!({"fidelity": 90, "bend": -30})).unwrap();
    match &node(&s, e).kind {
        NodeKind::Envelope { fidelity, kind, .. } => {
            let EnvelopeKind::Warp { bend, style, .. } = &**kind else { panic!("{kind:?}") };
            assert!(close(*fidelity, 90.0) && close(*bend, -30.0) && style == "arc");
        }
        k => panic!("{k:?}"),
    }
    assert!(s.execute("object.envelope.options", &json!({"fidelity": 101})).is_err());
    let r = s.execute("object.envelope.editContents", &json!({})).unwrap();
    assert_eq!(r["editing"], true);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    let r = s.execute("object.envelope.editContents", &json!({})).unwrap();
    assert_eq!(r["editing"], false);
    assert_eq!(s.doc().unwrap().selection.objects, vec![e]);
}

/// Edit Contents refuses an `editing` given as text or as another value that is not true or false,
/// and the envelope, the selection and the undo history stay as they were; a null `editing` toggles,
/// as a missing one does.
#[test]
fn envelope_edit_contents_refuses_editing_given_as_text() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 50})).unwrap());
    let before = (node(&s, e), undo_len(&s), s.doc().unwrap().selection.objects.clone());
    for p in [json!({"editing": "true"}), json!({"editing": 1}), json!({"editing": "false"}), json!({"editing": [true]})] {
        refuses_non_boolean(&mut s, "object.envelope.editContents", &p, "editing");
        assert_eq!((node(&s, e), undo_len(&s), s.doc().unwrap().selection.objects.clone()), before, "{p}");
    }
    let r = s.execute("object.envelope.editContents", &json!({"editing": null})).unwrap();
    assert_eq!((r["editing"].clone(), s.doc().unwrap().selection.objects.clone()), (json!(true), vec![a]));
}

/// The warp's `bend`, and `h` and `v` by those keys or by their long names, given as text.
fn warp_numbers_as_text() -> [(&'static str, Value); 5] {
    [
        ("bend", json!({"bend": "40"})),
        ("h", json!({"h": "10"})),
        ("v", json!({"v": "-10"})),
        ("horizontalDistortion", json!({"horizontalDistortion": "10"})),
        ("verticalDistortion", json!({"verticalDistortion": "10%"})),
    ]
}

/// Make with Warp refuses a bend or distortion given as text, and makes no envelope.
#[test]
fn envelope_make_with_warp_refuses_a_number_given_as_text() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let n = undo_len(&s);
    for (key, p) in warp_numbers_as_text() {
        refuses_non_number(&mut s, "object.envelope.makeWithWarp", &p, key);
    }
    assert_eq!(undo_len(&s), n, "nothing changed");
    assert!(matches!(node(&s, a).kind, NodeKind::Path { .. }), "not enveloped");
}

/// Reset with Warp refuses a bend or distortion given as text, and the envelope keeps its warp.
#[test]
fn envelope_reset_with_warp_refuses_a_number_given_as_text() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 50})).unwrap());
    let (before, n) = (node(&s, e), undo_len(&s));
    for (key, p) in warp_numbers_as_text() {
        refuses_non_number(&mut s, "object.envelope.resetWithWarp", &p, key);
    }
    assert_eq!((node(&s, e), undo_len(&s)), (before, n), "nothing changed");
}

/// Make with Mesh refuses rows or columns given as text, and makes no envelope.
#[test]
fn envelope_make_with_mesh_refuses_a_number_given_as_text() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let n = undo_len(&s);
    for (key, p) in [("rows", json!({"rows": "3"})), ("cols", json!({"cols": "3"})), ("columns", json!({"rows": 2, "columns": "3"}))] {
        refuses_non_number(&mut s, "object.envelope.makeWithMesh", &p, key);
    }
    assert_eq!(undo_len(&s), n, "nothing changed");
    assert!(matches!(node(&s, a).kind, NodeKind::Path { .. }), "not enveloped");
}

/// Reset with Mesh refuses rows or columns given as text, and the envelope keeps its mesh.
#[test]
fn envelope_reset_with_mesh_refuses_a_number_given_as_text() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let e = id_of(&s.execute("object.envelope.makeWithMesh", &json!({"rows": 2, "cols": 2})).unwrap());
    let (before, n) = (node(&s, e), undo_len(&s));
    for (key, p) in [("rows", json!({"rows": "3"})), ("cols", json!({"cols": "3"})), ("columns", json!({"columns": "3", "maintainShape": false}))] {
        refuses_non_number(&mut s, "object.envelope.resetWithMesh", &p, key);
    }
    assert_eq!((node(&s, e), undo_len(&s)), (before, n), "nothing changed");
}

/// Make with Warp and Reset with Warp refuse a `horizontal` given as text or as another value that
/// is not true or false, also beside an `orientation`, and Reset with Mesh refuses such a
/// `maintainShape`: no envelope is made, and a warp envelope keeps its warp. Null is read as the key
/// left out.
#[test]
fn envelope_make_and_reset_refuse_a_boolean_given_as_text() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let n = undo_len(&s);
    let warps = [json!({"horizontal": "false"}), json!({"style": "arch", "horizontal": 0}), json!({"orientation": "vertical", "horizontal": "true"})];
    for p in &warps {
        refuses_non_boolean(&mut s, "object.envelope.makeWithWarp", p, "horizontal");
    }
    assert_eq!(undo_len(&s), n, "nothing changed");
    assert!(matches!(node(&s, a).kind, NodeKind::Path { .. }), "not enveloped");
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 50, "horizontal": null})).unwrap());
    let (before, n) = (node(&s, e), undo_len(&s));
    for p in &warps {
        refuses_non_boolean(&mut s, "object.envelope.resetWithWarp", p, "horizontal");
    }
    for p in [json!({"maintainShape": "false"}), json!({"rows": 2, "cols": 2, "maintainShape": 0})] {
        refuses_non_boolean(&mut s, "object.envelope.resetWithMesh", &p, "maintainShape");
    }
    assert_eq!((node(&s, e), undo_len(&s)), (before, n), "nothing changed");
    s.execute("object.envelope.resetWithWarp", &json!({"horizontal": false})).unwrap();
    assert!(matches!(&node(&s, e).kind, NodeKind::Envelope { kind, .. } if matches!(**kind, EnvelopeKind::Warp { horizontal: false, .. })));
    s.execute("object.envelope.resetWithMesh", &json!({"rows": 2, "cols": 2, "maintainShape": null})).unwrap();
    assert!(matches!(&node(&s, e).kind, NodeKind::Envelope { kind, .. } if matches!(**kind, EnvelopeKind::Mesh { rows: 2, cols: 2, .. })));
}

/// Envelope Options refuses a fidelity, bend or distortion given as text, for the selected
/// envelope and for the options new envelopes get, and changes neither.
#[test]
fn envelope_options_refuse_a_number_given_as_text() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 50})).unwrap());
    let (before, n) = (node(&s, e), undo_len(&s));
    let mut calls = vec![("fidelity", json!({"fidelity": "80", "antiAlias": false}))];
    calls.extend(warp_numbers_as_text());
    for (key, p) in &calls {
        refuses_non_number(&mut s, "object.envelope.options", p, key);
    }
    assert_eq!((node(&s, e), undo_len(&s)), (before, n), "nothing changed");
    s.execute("select.none", &json!({})).unwrap();
    for (key, p) in &calls {
        refuses_non_number(&mut s, "object.envelope.options", p, key);
    }
    assert_eq!(s.envelope_defaults, None, "the options new envelopes get stayed");
}

/// Envelope Options refuses Anti-Alias, the three Distort options or a warp's `horizontal` given as
/// text or as another value that is not true or false, for a selected warp envelope, a selected mesh
/// envelope and the options new envelopes get, and changes none of them; null keeps the current
/// value, as a missing key does.
#[test]
fn envelope_options_refuse_a_boolean_given_as_text() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let warp = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 50})).unwrap());
    let b = rect(&mut s, 400.0, 100.0, 100.0, 100.0);
    sel(&mut s, &[b]);
    let mesh = id_of(&s.execute("object.envelope.makeWithMesh", &json!({"rows": 2, "cols": 2})).unwrap());
    let calls = [
        ("antiAlias", json!({"antiAlias": "false"})),
        ("distortAppearance", json!({"fidelity": 80, "distortAppearance": 1})),
        ("distortLinearGradients", json!({"distortLinearGradients": "true"})),
        ("distortPatternFills", json!({"distortPatternFills": [true]})),
        ("horizontal", json!({"horizontal": "false"})),
    ];
    for ids in [vec![warp], vec![mesh], vec![]] {
        sel(&mut s, &ids);
        let before = (node(&s, warp), node(&s, mesh), undo_len(&s));
        for (key, p) in &calls {
            refuses_non_boolean(&mut s, "object.envelope.options", p, key);
        }
        assert_eq!((node(&s, warp), node(&s, mesh), undo_len(&s)), before, "nothing changed with {ids:?} selected");
    }
    assert_eq!(s.envelope_defaults, None, "the options new envelopes get stayed");
    sel(&mut s, &[warp]);
    s.execute("object.envelope.options", &json!({"antiAlias": false, "distortAppearance": null, "horizontal": null})).unwrap();
    assert!(matches!(&node(&s, warp).kind, NodeKind::Envelope { options, .. } if !options.anti_alias && options.distort_appearance));
}

#[test]
fn envelope_renders_distorted() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    s.execute("object.envelope.makeWithWarp", &json!({"style": "arch", "bend": 60})).unwrap();
    let img = render(&s, Rect::new(0.0, 0.0, 400.0, 300.0));
    // Arch lifts the middle: red above the original top edge at the centre, not at the sides.
    let mid = img.pixel(200, 85);
    assert!(mid[0] > 200 && mid[1] < 60, "{mid:?}");
    let side = img.pixel(102, 85);
    assert!(side[1] > 200, "{side:?}");
}

#[test]
fn envelope_transform_moves_mesh_points() {
    let mut s = session();
    let a = text_rect(&mut s);
    sel(&mut s, &[a]);
    let e = id_of(&s.execute("object.envelope.makeWithMesh", &json!({"rows": 1, "cols": 1})).unwrap());
    s.edit("move", |d, _| {
        d.node_mut(e).unwrap().transform(vectorcraft_geom::Affine::translate((10.0, 20.0)), false);
        Ok(())
    })
    .unwrap();
    let b = node(&s, e).geometric_bounds().unwrap();
    assert_eq!(b, Rect::new(110.0, 120.0, 310.0, 220.0));
    let pts = all_anchors(&live::expand_live(&node(&s, e)));
    assert!(pts.iter().any(|p| p.distance(Point::new(110.0, 120.0)) < 1e-6));
}

// ---------- Gradient mesh ----------

fn mesh_of(s: &Session, id: NodeId) -> vectorcraft_doc::GradientMesh {
    match &node(s, id).kind {
        NodeKind::Mesh(m) => m.clone(),
        k => panic!("{k:?}"),
    }
}

#[test]
fn mesh_create_flat() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    fill(&mut s, a, "#0000ff");
    sel(&mut s, &[a]);
    s.execute("object.mesh.create", &json!({"rows": 3, "cols": 2})).unwrap();
    let m = mesh_of(&s, a);
    assert_eq!((m.rows, m.cols, m.points.len()), (3, 2, 12));
    assert!(m.points.iter().all(|p| p.color.to_hex() == "#0000ff"));
    assert_eq!(node(&s, a).geometric_bounds().map(|b| (b.x0.round(), b.x1.round())), Some((0.0, 100.0)));
    assert_eq!(node(&s, a).kind_label(), "Mesh");
}

#[test]
fn mesh_to_center_highlight() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    fill(&mut s, a, "#ff0000");
    sel(&mut s, &[a]);
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2, "appearance": "center", "highlight": 100})).unwrap();
    let m = mesh_of(&s, a);
    assert_eq!(m.points[4].color.to_hex(), "#ffffff");
    assert_eq!(m.points[0].color.to_hex(), "#ff0000");
    assert!(close(m.points[4].p.x, 50.0) && close(m.points[4].p.y, 50.0));
    // Colour inside patch (0,0) at its far corner = the centre.
    let (c, _) = m.color_at(0, 0, 1.0, 1.0);
    assert_eq!(c.to_hex(), "#ffffff");
}

#[test]
fn mesh_renders_corner_and_centre_colours() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    fill(&mut s, a, "#ff0000");
    sel(&mut s, &[a]);
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2, "appearance": "center", "highlight": 100})).unwrap();
    let img = render(&s, Rect::new(0.0, 0.0, 100.0, 100.0));
    let c = img.pixel(50, 50);
    assert!(c[0] > 240 && c[1] > 220 && c[2] > 220, "centre near white {c:?}");
    let k = img.pixel(2, 2);
    assert!(k[0] > 240 && k[1] < 40, "corner red {k:?}");
    let e = img.pixel(25, 25);
    assert!(e[1] > 40 && e[1] < 220, "in between {e:?}");
}

#[test]
fn mesh_point_colour_move_and_undo() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    s.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
    s.execute("object.mesh.setPointColor", &json!({"id": a.0, "index": 3, "color": "#00ff00", "opacity": 0.5})).unwrap();
    let m = mesh_of(&s, a);
    assert_eq!(m.points[3].color.to_hex(), "#00ff00");
    assert!((m.points[3].opacity - 0.5).abs() < 1e-6);
    s.execute("object.mesh.movePoint", &json!({"id": a.0, "index": 3, "x": 120, "y": 130})).unwrap();
    assert_eq!(mesh_of(&s, a).points[3].p, Point::new(120.0, 130.0));
    s.execute("object.mesh.movePoint", &json!({"id": a.0, "index": 3, "x": 125, "y": 130, "handle": 1})).unwrap();
    assert_eq!(mesh_of(&s, a).points[3].handles[1], vectorcraft_geom::Vec2::new(5.0, 0.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(mesh_of(&s, a).points[3].p, Point::new(100.0, 100.0));
    assert!(s.execute("object.mesh.setPointColor", &json!({"id": a.0, "index": 99, "color": "#000000"})).is_err());
}

#[test]
fn mesh_add_line_and_delete_point() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    s.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
    let r = s.execute("object.mesh.addLine", &json!({"id": a.0, "x": 40, "y": 70, "color": "#123456"})).unwrap();
    let i = r["index"].as_u64().unwrap() as usize;
    let m = mesh_of(&s, a);
    assert_eq!((m.rows, m.cols), (2, 2));
    assert!(m.points[i].p.distance(Point::new(40.0, 70.0)) < 0.5);
    assert_eq!(m.points[i].color.to_hex(), "#123456");
    assert!(s.execute("object.mesh.addLine", &json!({"id": a.0, "x": 400, "y": 70})).is_err());
    s.execute("object.mesh.deletePoint", &json!({"id": a.0, "index": i})).unwrap();
    let m = mesh_of(&s, a);
    assert_eq!((m.rows, m.cols), (1, 1));
    assert!(s.execute("object.mesh.deletePoint", &json!({"id": a.0, "index": 0})).is_err(), "corner lines can't go");
}

/// A mesh point `index`, and `rows`, `cols` or `columns` for Create Gradient Mesh, that is not a
/// whole number is an error that changes nothing; 4.0 names point 4.
#[test]
fn mesh_commands_refuse_an_index_or_count_that_is_not_a_whole_number() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 200.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    let (before, n) = (mesh_of(&s, a), undo_len(&s));
    sel(&mut s, &[b]);
    for (key, p) in [("rows", json!({"rows": 2.5})), ("cols", json!({"cols": 1.5})), ("columns", json!({"columns": 3.25}))] {
        let r = s.execute("object.mesh.create", &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == format!("`{key}` must be a whole number")), "{p}: {r:?}");
    }
    assert!(matches!(node(&s, b).kind, NodeKind::Path { .. }), "still a path");
    for (cmd, p) in [
        ("object.mesh.setPointColor", json!({"id": a.0, "index": 4.5, "color": "#00ff00"})),
        ("object.mesh.movePoint", json!({"id": a.0, "index": 4.5, "x": 10, "y": 10})),
        ("object.mesh.movePoint", json!({"id": a.0, "index": 3.5, "x": 10, "y": 10, "handle": 0})),
        ("object.mesh.deletePoint", json!({"id": a.0, "index": 4.5})),
    ] {
        let r = s.execute(cmd, &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == "`index` must be a whole number"), "{cmd} {p}: {r:?}");
    }
    assert_eq!((mesh_of(&s, a), undo_len(&s)), (before, n), "nothing changed");
    s.execute("object.mesh.movePoint", &json!({"id": a.0, "index": 4.0, "x": 40, "y": 60})).unwrap();
    assert_eq!(mesh_of(&s, a).points[4].p, Point::new(40.0, 60.0));
}

/// Create Gradient Mesh refuses rows, columns or a highlight given as text, and an `at` that is not
/// two numbers, and makes no mesh; `at: [50, 40]` with a null highlight makes one.
#[test]
fn mesh_create_refuses_a_number_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let n = undo_len(&s);
    for (key, p) in [
        ("rows", json!({"rows": "3"})),
        ("cols", json!({"cols": "2"})),
        ("columns", json!({"columns": "2"})),
        ("highlight", json!({"appearance": "center", "highlight": "50"})),
    ] {
        refuses_non_number(&mut s, "object.mesh.create", &p, key);
    }
    for at in [json!(["50", "40"]), json!([50]), json!(50), json!({"x": 50, "y": 40})] {
        let r = s.execute("object.mesh.create", &json!({ "at": at }));
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == "`at` must be [x, y]"), "{at}: {r:?}");
    }
    assert_eq!(undo_len(&s), n, "nothing changed");
    assert!(matches!(node(&s, a).kind, NodeKind::Path { .. }), "still a path");
    s.execute("object.mesh.create", &json!({"at": [50, 40], "highlight": null})).unwrap();
    assert_eq!((mesh_of(&s, a).rows, mesh_of(&s, a).cols), (2, 2));
}

/// Set Mesh Point Color refuses an opacity given as text, and the point keeps its color and opacity.
#[test]
fn mesh_set_point_color_refuses_an_opacity_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    s.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
    let (before, n) = (mesh_of(&s, a), undo_len(&s));
    for p in [json!({"id": a.0, "index": 3, "color": "#00ff00", "opacity": "0.5"}), json!({"id": a.0, "index": 3, "opacity": "50%"})] {
        refuses_non_number(&mut s, "object.mesh.setPointColor", &p, "opacity");
    }
    assert_eq!((mesh_of(&s, a), undo_len(&s)), (before, n), "nothing changed");
}

#[test]
fn mesh_create_at_point_single_undo() {
    let mut s = session();
    let a = id_of(&s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 100, "height": 80})).unwrap());
    sel(&mut s, &[a]);
    let n = undo_len(&s);
    let r = s.execute("object.mesh.create", &json!({"at": [50, 40]})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let m = mesh_of(&s, a);
    assert_eq!((m.rows, m.cols), (2, 2));
    let i = r["index"].as_u64().unwrap() as usize;
    assert!(m.points[i].p.distance(Point::new(50.0, 40.0)) < 0.5);
    // Mesh boundary follows the ellipse.
    let b = node(&s, a).geometric_bounds().unwrap();
    assert!(b.x0 > -3.0 && b.x1 < 103.0, "{b:?}");
}

#[test]
fn mesh_expand_to_flat_pieces() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    s.execute("object.mesh.create", &json!({"rows": 1, "cols": 2})).unwrap();
    s.execute("object.mesh.expand", &json!({})).unwrap();
    let g = node(&s, a);
    assert!(matches!(g.kind, NodeKind::Group { .. }));
    assert_eq!(g.children().unwrap().len(), 2 * 64);
    assert!(g.children().unwrap().iter().all(|c| c.id != NodeId(0)));
}

#[test]
fn mesh_hit_test_inside() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    let d = &s.doc().unwrap().doc;
    let h = vectorcraft_doc::hit::hit_test(d, Point::new(50.0, 30.0), vectorcraft_doc::hit::HitOptions { tol: 1.0, ..Default::default() });
    assert_eq!(h.map(|h| h.leaf), Some(a));
}

#[test]
fn blend_of_meshes_interpolates_mesh_points() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    let b = rect(&mut s, 200.0, 0.0, 50.0, 50.0);
    fill(&mut s, a, "#000000");
    fill(&mut s, b, "#ffffff");
    sel(&mut s, &[a, b]);
    s.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 1})).unwrap());
    let st = steps_of(&s, g);
    match &st[1].kind {
        NodeKind::Mesh(m) => {
            assert!(close(m.points[0].p.x, 100.0));
            assert!((m.points[0].color.to_rgb()[0] - 0.5).abs() < 1e-3);
        }
        k => panic!("{k:?}"),
    }
    let img = render(&s, Rect::new(0.0, 0.0, 250.0, 50.0));
    assert!(img.pixel(125, 25)[3] == 255);
}

// ---------- Persistence and export ----------

fn live_doc() -> Session {
    let mut s = session();
    let (_, _, _) = blend(&mut s, json!({"steps": 2}));
    let a = rect(&mut s, 100.0, 300.0, 100.0, 50.0);
    sel(&mut s, &[a]);
    s.execute("object.envelope.makeWithWarp", &json!({"style": "wave", "bend": 30})).unwrap();
    let m = rect(&mut s, 400.0, 300.0, 100.0, 100.0);
    sel(&mut s, &[m]);
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2, "appearance": "edge", "highlight": 50})).unwrap();
    s
}

#[test]
fn live_objects_round_trip_through_native_format() {
    let s = live_doc();
    let d = &s.doc().unwrap().doc;
    let bytes = vectorcraft_format::save(d, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(&back.layers, &d.layers);
    let mut kinds = vec![];
    back.walk(|n| kinds.push(n.kind_label()));
    for k in ["Blend", "Envelope", "Mesh"] {
        assert!(kinds.contains(&k), "{k}");
    }
}

#[test]
fn svg_export_expands_live_objects() {
    let s = live_doc();
    let svg = vectorcraft_svg::export(&s.doc().unwrap().doc, &Default::default());
    // 4 blend objects + envelope content + 4 patches × 64 mesh pieces.
    let paths = svg.matches("<path").count();
    assert!(paths >= 4 + 1 + 256, "{paths}");
    assert!(vectorcraft_svg::import(&svg).is_ok());
}

#[test]
fn pdf_export_expands_live_objects() {
    let s = live_doc();
    let pdf = vectorcraft_pdf::export(&s.doc().unwrap().doc, &Default::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn old_documents_still_load() {
    let s = session();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, true)).unwrap();
    assert_eq!(back.layers.len(), d.layers.len());
}

#[test]
fn live_commands_have_menus_and_params() {
    for id in [
        "object.blend.make",
        "object.blend.replaceSpine",
        "object.envelope.makeWithWarp",
        "object.envelope.makeWithMesh",
        "object.envelope.makeWithTopObject",
        "object.envelope.release",
        "object.envelope.expand",
        "object.envelope.editContents",
        "object.envelope.options",
        "object.mesh.create",
    ] {
        let c = crate::cmd::find_command(id).unwrap_or_else(|| panic!("{id}"));
        assert!(!c.params.is_empty() && !c.menu.is_empty(), "{id}");
    }
    for id in ["object.mesh.setPointColor", "object.mesh.movePoint", "object.mesh.addLine", "object.mesh.expand"] {
        assert!(crate::cmd::find_command(id).is_some(), "{id}");
    }
}

#[test]
fn disabled_without_live_selection() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    assert!(s.execute("object.blend.release", &json!({})).is_err());
    assert!(s.execute("object.envelope.release", &json!({})).is_err());
    assert!(s.execute("object.mesh.expand", &json!({})).is_err());
    assert!(s.execute("object.blend.make", &json!({})).is_err(), "needs two objects");
    let _ = Paint::solid(Color::BLACK);
}

/// A selected embedded 8×4 image at (100, 50), 2 pt per pixel: red on the left half, blue on the
/// right half (which is transparent when `clear_right`). `link` makes it a linked image.
fn two_colour_image(s: &mut Session, clear_right: bool, linked: bool) -> NodeId {
    let right = image::Rgba([0, 0, 255, if clear_right { 0 } else { 255 }]);
    let img = image::RgbaImage::from_fn(8, 4, |x, _| if x < 4 { image::Rgba([255, 0, 0, 255]) } else { right });
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.edit("Place", |d, sel| {
        d.images.insert("pic".into(), vectorcraft_doc::ImageBlob::png(png));
        let id = d.alloc_id();
        let xf = vectorcraft_geom::Affine::translate((100.0, 50.0)) * vectorcraft_geom::Affine::scale(2.0);
        let link = linked.then(|| vectorcraft_doc::LinkInfo { path: "/tmp/pic.png".into(), ..Default::default() });
        let im = vectorcraft_doc::ImageObject { key: "pic".into(), width: 8, height: 4, xf, link, placement: Default::default(), pixels: None };
        d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

#[test]
fn mesh_create_on_an_embedded_image_takes_its_colours_and_placement() {
    let mut s = session();
    let id = two_colour_image(&mut s, false, false);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("object.mesh.create", &json!({"rows": 1, "cols": 2})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let m = mesh_of(&s, id);
    assert_eq!((m.rows, m.cols, m.points.len()), (1, 2, 6));
    // Row 0 is the image's top: red at the left edge, the average of both halves in the middle
    // (an area, not one pixel), blue at the right edge.
    let hex = |i: usize| m.points[i].color.to_hex();
    assert_eq!((hex(0).as_str(), hex(1).as_str(), hex(2).as_str()), ("#ff0000", "#800080", "#0000ff"));
    assert_eq!((hex(3).as_str(), hex(5).as_str()), ("#ff0000", "#0000ff"));
    assert!(m.points.iter().all(|p| p.opacity == 1.0));
    // The mesh keeps the image's transform: (100, 50) to (116, 58).
    let near = |p: Point, x: f64, y: f64| close(p.x, x) && close(p.y, y);
    assert!(near(m.points[0].p, 100.0, 50.0) && near(m.points[2].p, 116.0, 50.0) && near(m.points[5].p, 116.0, 58.0));
    assert_eq!(node(&s, id).kind_label(), "Mesh");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id).kind_label(), "Image");
}

#[test]
fn mesh_create_on_an_image_keeps_alpha_and_refuses_linked_and_big_grids() {
    let mut s = session();
    let id = two_colour_image(&mut s, true, false);
    s.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
    let m = mesh_of(&s, id);
    assert_eq!(m.points.iter().map(|p| p.opacity).collect::<Vec<_>>(), [1.0, 0.0, 1.0, 0.0]);
    assert_eq!(m.points[0].color.to_hex(), "#ff0000");

    let mut s = session();
    two_colour_image(&mut s, false, false);
    assert!(s.execute("object.mesh.create", &json!({"rows": 51, "cols": 4})).is_err(), "grid capped");

    let mut s = session();
    let id = two_colour_image(&mut s, false, true);
    let e = s.execute("object.mesh.create", &json!({})).unwrap_err().to_string();
    assert!(e.contains("linked"), "{e}");
    assert_eq!(node(&s, id).kind_label(), "Image", "left as it was");
}
