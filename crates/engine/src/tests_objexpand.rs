//! Object → Expand (M3.84): the Object, Fill and Stroke options, gradient fills into solid strips,
//! concentric ellipses or a gradient mesh inside a clip group, freeform gradients into a mesh, and
//! Create Gradient Mesh taking a gradient fill's colours.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Rect;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

/// A selected rectangle filled with `gradient` (paint.setFill's), with no stroke unless `stroke`.
fn gradient_rect(s: &mut Session, r: Rect, gradient: Value, stroke: bool) -> NodeId {
    let id = run(s, "shape.rectangle", json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()}))["id"].as_u64().unwrap();
    run(s, "paint.setFill", json!({ "gradient": gradient }));
    if stroke {
        run(s, "paint.setStroke", json!({"color": "#0000ff"}));
        run(s, "stroke.set", json!({"weight": 4}));
    } else {
        run(s, "paint.setStroke", json!({"none": true}));
    }
    NodeId(id)
}

/// The one selected object after a command.
fn selected(s: &Session) -> Node {
    let sel = &s.doc().unwrap().selection.objects;
    assert_eq!(sel.len(), 1, "one object selected");
    node(s, sel[0])
}

/// A clip group's clipping path and contents.
fn clip_parts(n: &Node) -> (&Node, Vec<&Node>) {
    match &n.kind {
        NodeKind::Group { clip: true, children } => (&children[0], children[1..].iter().map(|c| &**c).collect()),
        k => panic!("expected a clip group, got {k:?}"),
    }
}

fn fill_color(n: &Node) -> Color {
    n.appearance.fill_paint().color().unwrap_or_else(|| panic!("solid fill expected on {:?}", n.kind))
}

fn render(s: &Session) -> vectorcraft_render::Rendered {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts)
}

/// The mean difference of the colour channels of two renders.
fn mean_diff(a: &vectorcraft_render::Rendered, b: &vectorcraft_render::Rendered) -> f64 {
    let sum: u64 = a.pixels.iter().zip(b.pixels.iter()).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum as f64 / a.pixels.len() as f64
}

/// Pixels (of 200×200) whose colour differs by more than `tol` in any channel.
fn differing(a: &vectorcraft_render::Rendered, b: &vectorcraft_render::Rendered, tol: u8) -> usize {
    a.pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > tol))
        .count()
}

fn white_black() -> Value {
    json!({"kind": "linear", "stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]})
}

#[test]
fn eight_strips_match_the_gradient_samples() {
    let mut s = session();
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), white_black(), false);
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "object.expand", json!({"steps": 8}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let g = selected(&s);
    let (clip, strips) = clip_parts(&g);
    assert!(matches!(clip.kind, NodeKind::Path { clipping: true, .. }));
    assert_eq!(clip.geometric_bounds().unwrap(), Rect::new(20.0, 20.0, 180.0, 120.0));
    assert_eq!(strips.len(), 8);
    for (k, strip) in strips.iter().enumerate() {
        // Band k starts at its eighth of the width (and runs on under the next bands, so no
        // seams show) and shows the gradient at its middle.
        let b = strip.geometric_bounds().unwrap();
        assert!((b.x0 - (20.0 + 20.0 * k as f64)).abs() < 1e-9, "strip {k} starts at {}", b.x0);
        assert!((b.x1 - 180.0).abs() < 1e-9 && (b.y0 - 20.0).abs() < 1e-9 && (b.y1 - 120.0).abs() < 1e-9);
        let gray = 1.0 - (k as f32 + 0.5) / 8.0;
        let [r, g, bl] = fill_color(strip).to_rgb();
        assert!((r - gray).abs() < 1e-4 && (g - gray).abs() < 1e-4 && (bl - gray).abs() < 1e-4, "strip {k}: {r} {g} {bl} vs {gray}");
    }
    run(&mut s, "edit.undo", json!({}));
    assert!(matches!(selected(&s).appearance.fill_paint(), Paint::Gradient(_)));
}

#[test]
fn a_radial_gradient_becomes_concentric_ellipses() {
    let mut s = session();
    gradient_rect(&mut s, Rect::new(50.0, 50.0, 150.0, 150.0), json!({"kind": "radial"}), false);
    run(&mut s, "object.expand", json!({"steps": 4}));
    let g = selected(&s);
    let (_, rings) = clip_parts(&g);
    assert_eq!(rings.len(), 4);
    // Largest first, all centred on the gradient's centre; the largest reaches the box's corners.
    let reach = 50.0 * 2f64.sqrt();
    for (i, ring) in rings.iter().enumerate() {
        let b = ring.geometric_bounds().unwrap();
        assert!((b.center().x - 100.0).abs() < 1e-6 && (b.center().y - 100.0).abs() < 1e-6);
        let r = reach * (4 - i) as f64 / 4.0;
        assert!((b.width() / 2.0 - r).abs() < 1e-6 && (b.height() / 2.0 - r).abs() < 1e-6, "ring {i}: {b:?} vs radius {r}");
    }
    // The centre disc shows the gradient at the middle of the first band (white → black).
    let t = (reach / 8.0 / 50.0) as f32;
    let [r, ..] = fill_color(rings[3]).to_rgb();
    assert!((r - (1.0 - t)).abs() < 1e-4, "{r} vs {}", 1.0 - t);
    // Translucent stops: rings with holes instead of stacked discs.
    let mut s = session();
    let gr = json!({"kind": "radial", "stops": [{"offset": 0, "color": "#ff0000", "opacity": 0.5}, {"offset": 1, "color": "#0000ff"}]});
    gradient_rect(&mut s, Rect::new(50.0, 50.0, 150.0, 150.0), gr, false);
    run(&mut s, "object.expand", json!({"steps": 4}));
    let g = selected(&s);
    let (_, rings) = clip_parts(&g);
    let subpaths = |n: &Node| n.path_data().unwrap().subpaths.len();
    assert_eq!(rings.iter().map(|r| subpaths(r)).collect::<Vec<_>>(), [2, 2, 2, 1]);
    let Some(vectorcraft_doc::AppearanceItem::Fill(f)) = rings[3].appearance.items.first() else { panic!() };
    assert!(f.opacity < 1.0 && f.opacity > 0.5, "the centre band is translucent: {}", f.opacity);
}

#[test]
fn a_conic_gradient_becomes_wedges_or_a_mesh_that_paint_it() {
    let conic = json!({"kind": "conic", "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 0.5, "color": "#ff00ff"}, {"offset": 1, "color": "#004080"}]});
    // Objects: a wedge per step, each from the centre and coloured at its middle.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), conic.clone(), false);
    let before = render(&s);
    run(&mut s, "object.expand", json!({"steps": 180}));
    let g = selected(&s);
    let (_, wedges) = clip_parts(&g);
    assert_eq!(wedges.len(), 180);
    let first = wedges[0].path_data().unwrap().subpaths[0].anchors[0].p;
    assert!(first.distance(vectorcraft_geom::Point::new(100.0, 100.0)) < 1e-9, "wedges start at the centre: {first:?}");
    let after = render(&s);
    assert!(differing(&before, &after, 48) < 200, "{}", differing(&before, &after, 48));
    assert!(mean_diff(&before, &after) < 2.0, "{}", mean_diff(&before, &after));
    // Mesh: columns around the centre.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), conic, false);
    run(&mut s, "object.expand", json!({"gradient": "mesh"}));
    let g = selected(&s);
    let (_, content) = clip_parts(&g);
    let NodeKind::Mesh(m) = &content[0].kind else { panic!("a mesh node: {:?}", content[0].kind) };
    assert!(m.is_valid());
    let after = render(&s);
    assert!(differing(&before, &after, 48) < 200, "{}", differing(&before, &after, 48));
    assert!(mean_diff(&before, &after) < 2.5, "{}", mean_diff(&before, &after));
}

#[test]
fn the_mesh_option_makes_a_mesh_that_paints_the_gradient() {
    for gradient in [
        json!({"kind": "linear", "angle": 30, "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 0.4, "color": "#00ff00", "midpoint": 0.3}, {"offset": 1, "color": "#0000ff"}]}),
        json!({"kind": "radial", "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 0.5, "color": "#ff00ff"}, {"offset": 1, "color": "#004080"}]}),
    ] {
        let mut s = session();
        gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), gradient.clone(), false);
        let before = render(&s);
        run(&mut s, "object.expand", json!({"gradient": "mesh"}));
        let g = selected(&s);
        let (_, content) = clip_parts(&g);
        assert_eq!(content.len(), 1);
        let NodeKind::Mesh(m) = &content[0].kind else { panic!("a mesh node: {:?}", content[0].kind) };
        assert!(m.is_valid());
        // The mesh is drawn as small flat pieces, so it is close but not identical.
        let after = render(&s);
        assert_eq!(differing(&before, &after, 48), 0, "{gradient}");
        assert!(mean_diff(&before, &after) < 2.5, "{gradient}: {}", mean_diff(&before, &after));
    }
}

#[test]
fn a_hard_stop_keeps_its_edge_in_the_mesh() {
    let mut s = session();
    let gr = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 0.5, "color": "#ff0000"}, {"offset": 0.5, "color": "#0000ff"}, {"offset": 1, "color": "#0000ff"}]});
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), gr, false);
    run(&mut s, "object.expand", json!({"gradient": "mesh"}));
    let g = selected(&s);
    let (_, content) = clip_parts(&g);
    let NodeKind::Mesh(m) = &content[0].kind else { panic!() };
    // Columns at 0, 0.5 (red), 0.5 (blue), 1: the middle patch has no width.
    assert_eq!(m.cols, 3);
    let (a, b) = (&m.points[m.idx(0, 1)], &m.points[m.idx(0, 2)]);
    assert!(a.p.distance(b.p) < 1e-9);
    assert_eq!((a.color.to_hex(), b.color.to_hex()), ("#ff0000".into(), "#0000ff".into()));
}

#[test]
fn a_freeform_gradient_expands_to_a_mesh_shaped_like_the_object() {
    let mut s = session();
    let gr = json!({"kind": "freeform", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 0.5, "color": "#00ff00"}, {"offset": 1, "color": "#0000ff"}]});
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), gr, false);
    let before = render(&s);
    // Even with Specify … Objects.
    run(&mut s, "object.expand", json!({"gradient": "objects"}));
    let n = selected(&s);
    let NodeKind::Mesh(m) = &n.kind else { panic!("a mesh: {:?}", n.kind) };
    assert!(m.rows > 1 && m.cols > 1);
    let b = m.outline().bounds().unwrap();
    assert!((b.x0 - 20.0).abs() < 1e-6 && (b.y0 - 20.0).abs() < 1e-6 && (b.x1 - 180.0).abs() < 1e-6 && (b.y1 - 180.0).abs() < 1e-6, "{b:?}");
    let after = render(&s);
    assert!(mean_diff(&before, &after) < 2.5, "{}", mean_diff(&before, &after));
}

#[test]
fn create_gradient_mesh_takes_its_colours_from_the_gradient() {
    let mut s = session();
    let id = gradient_rect(&mut s, Rect::new(0.0, 0.0, 100.0, 50.0), white_black(), false);
    run(&mut s, "object.mesh.create", json!({"rows": 1, "cols": 2}));
    let NodeKind::Mesh(m) = &node(&s, id).kind else { panic!() };
    let hex = |r: usize, c: usize| m.points[m.idx(r, c)].color.to_hex();
    for r in 0..=1 {
        assert_eq!(hex(r, 0), "#ffffff", "left column white");
        assert_eq!(hex(r, 2), "#000000", "right column black");
        assert_eq!(hex(r, 1), "#808080", "middle grey");
    }
    // A solid fill still gives one colour, the highlight still lightens.
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "object.mesh.create", json!({"rows": 2, "cols": 2, "appearance": "center", "highlight": 100}));
    let NodeKind::Mesh(m) = &node(&s, id).kind else { panic!() };
    assert_eq!(m.points[m.idx(0, 0)].color.to_hex(), "#ff0000");
    assert_eq!(m.points[m.idx(1, 1)].color.to_hex(), "#ffffff");
}

#[test]
fn expanded_colours_keep_the_stops_colour_model() {
    let mut s = session();
    let gr = json!({"kind": "linear", "stops": [{"offset": 0, "color": {"c": 1, "m": 0, "y": 0, "k": 0}}, {"offset": 1, "color": {"c": 0, "m": 1, "y": 0, "k": 0}}]});
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), gr, false);
    run(&mut s, "object.expand", json!({"steps": 4}));
    let g = selected(&s);
    let (_, strips) = clip_parts(&g);
    let Color::Cmyk { c, m, .. } = fill_color(strips[1]) else { panic!("CMYK strip expected: {:?}", fill_color(strips[1])) };
    assert!((c - 0.625).abs() < 1e-5 && (m - 0.375).abs() < 1e-5, "interpolated in CMYK: {c} {m}");
}

#[test]
fn fill_off_leaves_the_fill_live_and_stroke_off_keeps_the_stroke() {
    // Fill unchecked: the stroke is outlined, the gradient fill stays a gradient.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), white_black(), true);
    run(&mut s, "object.expand", json!({"fill": false}));
    let g = selected(&s);
    let NodeKind::Group { children, clip: false } = &g.kind else { panic!("fill + outlined stroke group: {:?}", g.kind) };
    assert_eq!(children.len(), 2);
    assert!(matches!(children[0].appearance.fill_paint(), Paint::Gradient(_)), "the fill is still live");
    assert_eq!(fill_color(&children[1]).to_hex(), "#0000ff", "the stroke became a filled outline");
    // Stroke unchecked: the gradient is expanded and the stroke stays a stroke on a copy above.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), white_black(), true);
    let before = render(&s);
    run(&mut s, "object.expand", json!({"stroke": false, "steps": 255}));
    let g = selected(&s);
    let NodeKind::Group { children, clip: false } = &g.kind else { panic!("{:?}", g.kind) };
    assert_eq!(clip_parts(&children[0]).1.len(), 255);
    assert_eq!(children[1].appearance.stroke_paint().color().unwrap().to_hex(), "#0000ff");
    assert!(children[1].appearance.fill_paint().is_none());
    assert_eq!(differing(&before, &render(&s), 2), 0, "looks the same");
}

#[test]
fn defaults_match_the_dialog_and_info_tells_what_applies() {
    let mut s = session();
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), white_black(), true);
    assert_eq!(run(&mut s, "object.expand.info", json!({})), json!({"object": true, "fill": true, "stroke": true}));
    let r = run(&mut s, "object.expand", json!({}));
    assert_eq!(r["ids"].as_array().unwrap().len(), 1);
    // Object, Fill and Stroke on; 255 objects: the live rectangle, its stroke and its gradient.
    let g = selected(&s);
    let NodeKind::Group { children, .. } = &g.kind else { panic!("{:?}", g.kind) };
    let (clip, strips) = clip_parts(&children[0]);
    assert!(matches!(clip.kind, NodeKind::Path { live: None, .. }));
    assert_eq!(strips.len(), 255);
    assert!(children[1].appearance.stroke_paint().is_none(), "outlined stroke");
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Expand");
    assert_eq!(run(&mut s, "object.expand.info", json!({})), json!({"object": false, "fill": false, "stroke": false}));
    assert!(s.execute("object.expand", &json!({})).is_err(), "nothing left to expand");
    // Bad options are refused before anything changes.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.execute("object.expand", &json!({"gradient": "rings"})).is_err());
    assert!(s.execute("object.expand", &json!({"steps": 0})).is_err());
    assert!(matches!(selected(&s).kind, NodeKind::Path { live: Some(_), .. }));
}
