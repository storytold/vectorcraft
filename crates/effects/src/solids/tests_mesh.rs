use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_doc::color::{Color, Gradient, GradientKind, GradientPaint, GradientStop};
use vectorcraft_geom::BezPath;

use super::*;

const RED: Color = Color::Rgb { r: 0.8, g: 0.2, b: 0.1 };

fn square_node(id: u64, x: f64) -> Node {
    let mut b = BezPath::new();
    b.move_to((x, 100.0));
    b.line_to((x + 100.0, 100.0));
    b.line_to((x + 100.0, 200.0));
    b.line_to((x, 200.0));
    b.close_path();
    Node::path(NodeId(id), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::solid(Color::BLACK), 4.0))
}

fn fx(id: &str, params: Value) -> Effect {
    crate::new_effect(id, &params).unwrap()
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

#[test]
fn every_3d_effect_gives_a_mesh_for_its_fill_and_its_stroke() {
    for id in SOLIDS {
        let mut n = square_node(1, 100.0);
        n.appearance.effects.push(fx(id, json!({})));
        let meshes = three_d_meshes(&n).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(meshes.len(), 2, "{id}: the fill's solid and the stroke's");
        assert!(close(meshes[0].color, [0.8, 0.2, 0.1, 1.0]), "{id}: {:?}", meshes[0].color);
        assert!(close(meshes[1].color, [0.0, 0.0, 0.0, 1.0]), "{id}: {:?}", meshes[1].color);
        assert!(meshes.iter().all(|m| !m.mesh.is_empty()), "{id}");
    }
}

#[test]
fn the_meshes_of_one_object_share_one_triangle_budget() {
    let mut n = square_node(1, 100.0);
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    let meshes = three_d_meshes(&n).unwrap();
    let first = meshes[0].mesh.triangles.len();
    assert!(first > 0);
    // Every mesh counts against what the ones before it used.
    let (mut out, mut total) = (vec![], 0);
    keep_mesh(&mut out, &mut total, meshes[0].clone(), first).unwrap();
    assert!(keep_mesh(&mut out, &mut total, meshes[1].clone(), first).is_err());
    assert_eq!(out.len(), 1);
    // Building past the budget stops with an error instead of holding every mesh.
    let shapes = [Profile { path: profile(&n).unwrap().0, rule: FillRule::NonZero, items: solid_paints(&n), opacity: 1.0 }];
    let Some(effect) = n.appearance.effects.first() else { panic!("no effect") };
    let mut total = 0;
    assert!(meshes_for(effect, &shapes, &mut total).is_ok());
    let mut total = MAX_MESH_TRIANGLES - first + 1;
    assert!(meshes_for(effect, &shapes, &mut total).is_err_and(|e| e.contains("too complex")));
}

#[test]
fn an_extruded_mesh_is_as_deep_as_the_effect() {
    let mut n = square_node(1, 100.0);
    n.appearance.items.truncate(1);
    n.appearance.effects.push(fx(EXTRUDE, json!({ "depth": 30.0, "rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0 })));
    let meshes = three_d_meshes(&n).unwrap();
    let (lo, hi) = meshes[0].mesh.bounds().unwrap();
    assert!((hi[2] - lo[2] - 30.0).abs() < 1e-9 && (hi[0] - lo[0] - 100.0).abs() < 1e-9, "{lo:?} {hi:?}");
}

#[test]
fn revolve_gives_a_mesh_in_the_top_fill() {
    let mut b = BezPath::new();
    b.move_to((100.0, 40.0));
    b.line_to((130.0, 60.0));
    b.line_to((130.0, 140.0));
    b.line_to((100.0, 160.0));
    let mut n = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::None, 0.0));
    n.appearance.effects.push(fx(crate::REVOLVE, json!({})));
    let meshes = three_d_meshes(&n).unwrap();
    assert_eq!(meshes.len(), 1);
    assert!(close(meshes[0].color, [0.8, 0.2, 0.1, 1.0]));
    assert!(meshes[0].mesh.triangles.len() >= 64 * 3, "three profile edges swept in 64 steps");
}

#[test]
fn an_object_without_a_3d_effect_has_no_mesh() {
    let mut n = square_node(1, 100.0);
    assert!(three_d_meshes(&n).is_err());
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    n.appearance.effects[0].visible = false;
    assert!(three_d_meshes(&n).is_err(), "a hidden effect builds nothing");
}

#[test]
fn the_object_and_the_paint_opacities_multiply() {
    let mut n = square_node(1, 100.0);
    n.appearance.items.truncate(1);
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.opacity = 0.5;
    }
    n.opacity = 0.5;
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    let meshes = three_d_meshes(&n).unwrap();
    assert!((meshes[0].color[3] - 0.25).abs() < 1e-6, "{:?}", meshes[0].color);
}

#[test]
fn an_effect_on_one_fill_gives_that_fills_mesh_alone() {
    let mut n = square_node(1, 100.0);
    if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.get_mut(1) {
        s.effects.push(fx(EXTRUDE, json!({})));
    }
    let meshes = three_d_meshes(&n).unwrap();
    assert_eq!(meshes.len(), 1);
    assert!(close(meshes[0].color, [0.0, 0.0, 0.0, 1.0]));
}

#[test]
fn a_group_gives_a_mesh_for_each_member() {
    let mut g = Node::group(NodeId(9), vec![Arc::new(square_node(1, 0.0)), Arc::new(square_node(2, 300.0))]);
    g.appearance.effects.push(fx(EXTRUDE, json!({ "rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0 })));
    let meshes = three_d_meshes(&g).unwrap();
    assert_eq!(meshes.len(), 4, "each member's fill and stroke");
    // The members turn about their shared centre: the first is left of it, the second right.
    let (lo, _) = meshes[0].mesh.bounds().unwrap();
    let (_, hi) = meshes[2].mesh.bounds().unwrap();
    assert!(lo[0] < -150.0 && hi[0] > 150.0, "{lo:?} {hi:?}");
}

#[test]
fn a_gradient_takes_the_average_of_its_stops() {
    let stops =
        vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop { opacity: 0.5, ..GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0)) }];
    let paint = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    assert!(close(mesh_color(&paint, 1.0), [0.5, 0.0, 0.5, 0.75]));
    let pattern = Paint::Pattern { pattern: "x".into(), xf: vectorcraft_geom::Affine::IDENTITY };
    assert!(close(mesh_color(&pattern, 0.5), [0.5, 0.5, 0.5, 0.5]));
}
