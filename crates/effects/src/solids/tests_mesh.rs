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
    let shapes = [Profile { path: profile(&n).unwrap().0, rule: FillRule::NonZero, items: solid_paints(&n), opacity: 1.0, scopes: vec![] }];
    let Some(effect) = n.appearance.effects.first() else { panic!("no effect") };
    let mut total = 0;
    assert!(meshes_for(effect, &shapes, &mut total, MAX_MESH_TRIANGLES).is_ok());
    let mut total = MAX_MESH_TRIANGLES - first + 1;
    assert!(meshes_for(effect, &shapes, &mut total, MAX_MESH_TRIANGLES).is_err_and(|e| e.contains("too complex")));
}

/// A mesh past the budget stops the export before the solids after it are built.
#[test]
fn the_solids_after_a_mesh_past_the_budget_are_not_built() {
    let mut n = square_node(1, 100.0);
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    let first = three_d_meshes(&n).unwrap()[0].mesh.triangles.len();
    let shapes = [Profile { path: profile(&n).unwrap().0, rule: FillRule::NonZero, items: solid_paints(&n), opacity: 1.0, scopes: vec![] }];
    let Some(effect) = n.appearance.effects.first() else { panic!("no effect") };
    let mut total = MAX_MESH_TRIANGLES - first + 1;
    SOLIDS_BUILT.with(|b| b.set(0));
    assert!(meshes_for(effect, &shapes, &mut total, MAX_MESH_TRIANGLES).is_err_and(|e| e.contains("too complex")));
    assert_eq!(SOLIDS_BUILT.with(std::cell::Cell::get), 1, "the stroke's solid is never built");
}

#[test]
fn revolve_stops_at_the_first_mesh_past_the_budget() {
    // The fill revolves into a mesh; the stroke's profile, copied 1000 times, is too complex to
    // revolve at all.
    let mut n = square_node(1, 100.0);
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.get_mut(0) {
        f.effects.push(fx(crate::REVOLVE, json!({})));
    }
    let fill_only = n.clone();
    if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.get_mut(1) {
        s.effects.push(fx("distort.transform", json!({ "moveH": 1.0, "copies": 1000 })));
        s.effects.push(fx(crate::REVOLVE, json!({})));
    }
    let fill = three_d_meshes(&fill_only).unwrap();
    let triangles = fill[0].mesh.triangles.len();
    assert!(triangles > 1);
    let whole = three_d_meshes(&n);
    assert!(whole.as_ref().is_err_and(|e| !e.contains("to export")), "the stroke's own error: {whole:?}");
    // With room for less than the fill's mesh, the stroke's is never built.
    let mut total = 0;
    let capped = meshes_within(None, &n, &mut total, triangles - 1);
    assert!(capped.as_ref().is_err_and(|e| e.contains("too complex to export")), "{capped:?}");
    assert_eq!(total, triangles);
}

#[test]
fn the_objects_of_one_asset_share_one_triangle_budget() {
    let mut a = square_node(1, 0.0);
    a.appearance.effects.push(fx(EXTRUDE, json!({})));
    let mut b = square_node(2, 300.0);
    b.appearance.effects.push(fx(EXTRUDE, json!({})));
    let triangles = |n: &Node| three_d_meshes(n).unwrap().iter().map(|m| m.mesh.triangles.len()).sum::<usize>();
    let (ta, tb) = (triangles(&a), triangles(&b));
    assert!(meshes_of_within(None, [&a], ta).is_ok());
    assert!(meshes_of_within(None, [&b], tb).is_ok());
    assert!(meshes_of_within(None, [&a, &b], ta.max(tb)).is_err_and(|e| e.contains("too complex to export")));
    assert_eq!(meshes_of_within(None, [&a, &b], ta + tb).map(|m| m.len()), Ok(4));
    assert_eq!(three_d_meshes_of([&a, &b]).map(|m| m.len()), Ok(4));
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

/// Extrude & Bevel (Classic) exports the solid it draws: its bevel as wide as it is high.
#[test]
fn a_classic_extrusion_exports_its_own_bevel() {
    let mut n = square_node(1, 100.0);
    n.appearance.items.truncate(1);
    let params = json!({ "bevel": "classic", "bevelHeight": 10.0 });
    n.appearance.effects.push(fx(CLASSIC_EXTRUDE, params.clone()));
    let meshes = three_d_meshes(&n).unwrap();
    let p = crate::merged_params(CLASSIC_EXTRUDE, &params);
    let path = profile(&n).unwrap().0;
    let bounds = path.bounds().unwrap();
    let own = vectorcraft_three_d::extrude_mesh(&path, FillRule::NonZero, bounds, classic_extrude_options(&p), view_options(&p)).unwrap();
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].mesh, own);
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

/// The meshes of Extrude & Bevel or Inflate `fx` on `shapes`, each built apart from the facets that
/// decide which solids are drawn ([`built_solids`], then [`vectorcraft_three_d::extrude_mesh`] or
/// [`vectorcraft_three_d::inflate_meshes`]), for comparing with [`meshes_for`].
fn meshes_built_apart(fx: &Effect, shapes: &[Profile], total: &mut usize, limit: usize) -> Result<Vec<SolidMesh>, String> {
    let p = crate::merged_params(&fx.id, &fx.params);
    let view = view_options(&p);
    let (parts, bounds) = solid_parts(shapes, union_bounds(shapes).unwrap());
    let drawn = built_solids(&fx.id, &p, &parts, bounds, MAX_OBJECT_FACES).map(|r| r.map(|f| f.is_some()));
    let mut inflated = (fx.id == INFLATE).then(|| {
        let shapes: Vec<(&PathData, FillRule)> = parts.iter().map(|(_, _, area, rule, _, _)| (area, *rule)).collect();
        vectorcraft_three_d::inflate_meshes(&shapes, bounds, inflate_options(&p), view)
    });
    let extrude = if fx.id == CLASSIC_EXTRUDE { classic_extrude_options(&p) } else { extrude_options(&p) };
    let mut out = vec![];
    for ((item, opacity, area, rule, _, _), drawn) in parts.iter().zip(drawn) {
        let item: &AppearanceItem = item;
        if !drawn? {
            if let Some(built) = inflated.as_mut() {
                built.next();
            }
            continue;
        }
        let mesh = match inflated.as_mut() {
            Some(built) => built.next().unwrap_or_else(|| Ok(Mesh::default())),
            None => vectorcraft_three_d::extrude_mesh(area, *rule, bounds, extrude, view),
        };
        match mesh {
            Ok(mesh) => {
                let (paint, opacity, _) = paint_of(item, *opacity);
                keep_mesh(&mut out, total, SolidMesh { mesh, color: mesh_color(&paint, opacity) }, limit)?;
            }
            Err(_) if matches!(item, AppearanceItem::Stroke(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// The bits of every number in `meshes`, for comparing two exports exactly.
type MeshBits = Vec<([u32; 4], Vec<[u64; 3]>, Vec<[u64; 3]>, Vec<[u32; 3]>)>;

fn mesh_bits(meshes: &[SolidMesh]) -> MeshBits {
    meshes
        .iter()
        .map(|m| {
            let (positions, normals) = (&m.mesh.positions, &m.mesh.normals);
            (
                m.color.map(f32::to_bits),
                positions.iter().map(|p| p.map(f64::to_bits)).collect(),
                normals.iter().map(|n| n.map(f64::to_bits)).collect(),
                m.mesh.triangles.clone(),
            )
        })
        .collect()
}

/// A five-pointed star about `(x, 150)` with curved edges, filled red and stroked black.
fn star_node(id: u64, x: f64) -> Node {
    let mut b = BezPath::new();
    for k in 0..10 {
        let (s, c) = (k as f64 * std::f64::consts::PI / 5.0).sin_cos();
        let r = if k % 2 == 0 { 60.0 } else { 25.0 };
        let p = (x + r * c, 150.0 + r * s);
        if k == 0 { b.move_to(p) } else { b.curve_to((p.0 + 4.0, p.1), (p.0, p.1 + 4.0), p) }
    }
    b.close_path();
    Node::path(NodeId(id), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::solid(Color::BLACK), 6.0))
}

/// The shapes `meshes_for` builds from: one node's, or each of several.
fn shapes_of(nodes: &[&Node]) -> Vec<Profile> {
    nodes
        .iter()
        .map(|n| Profile { path: profile(n).unwrap().0, rule: FillRule::NonZero, items: solid_paints(n), opacity: 1.0, scopes: vec![] })
        .collect()
}

/// Building each Extrude & Bevel and Inflate solid once exports the same meshes, to the bit, as
/// building its facets and its mesh apart, within and past the triangle budget.
#[test]
fn one_build_exports_the_meshes_built_apart() {
    let mut second_stroke = square_node(3, 400.0);
    if let Some(stroke) = second_stroke.appearance.items.get(1).cloned() {
        second_stroke.appearance.items.push(stroke);
    }
    let (star, square, other) = (star_node(1, 100.0), square_node(2, 250.0), star_node(4, 600.0));
    let cases = [
        (EXTRUDE, json!({})),
        (EXTRUDE, json!({ "bevel": "rounded", "bevelWidth": 6.0, "bevelHeight": 5.0, "twist": 60.0, "taper": 70.0 })),
        (EXTRUDE, json!({ "bevel": "cove", "bevelInside": true, "bevelBothSides": true, "cap": false, "perspectiveAngle": 50.0 })),
        (EXTRUDE, json!({ "depth": 0.0 })),
        (CLASSIC_EXTRUDE, json!({ "bevel": "classic", "bevelHeight": 8.0 })),
        (INFLATE, json!({})),
        (INFLATE, json!({ "depth": 30.0, "bothSides": true, "volume": 80.0 })),
        (INFLATE, json!({ "depth": 40.0, "twist": 360.0, "taper": 70.0, "cap": false })),
        (INFLATE, json!({ "depth": 0.1, "twist": 0.1, "bothSides": true })),
    ];
    let sets: [Vec<&Node>; 3] = [vec![&star], vec![&second_stroke], vec![&star, &square, &other]];
    for (id, params) in cases {
        let effect = fx(id, params.clone());
        for nodes in &sets {
            let shapes = shapes_of(nodes);
            let (mut once, mut apart) = (0, 0);
            let built = meshes_for(&effect, &shapes, &mut once, MAX_MESH_TRIANGLES).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
            let reference = meshes_built_apart(&effect, &shapes, &mut apart, MAX_MESH_TRIANGLES).unwrap();
            assert!(!reference.is_empty());
            assert_eq!(mesh_bits(&built), mesh_bits(&reference), "{id} {params}");
            assert_eq!(once, apart);
            // Past the budget, at the same mesh, with the same count.
            let limit = once - 1;
            let (mut once, mut apart) = (0, 0);
            let built = meshes_for(&effect, &shapes, &mut once, limit);
            assert!(built.is_err(), "{id} {params}");
            assert_eq!(built.err(), meshes_built_apart(&effect, &shapes, &mut apart, limit).err(), "{id} {params}");
            assert_eq!(once, apart, "{id} {params}");
        }
    }
}

/// A stroke whose solid is past the limits is left out of the export and the fill's mesh exports,
/// as when the facets and the mesh are built apart.
#[test]
fn a_stroke_past_the_limits_is_left_out_of_the_export() {
    // 40,000 corners: within the 65,536 points of the fill's solid, but not the stroke's outline.
    let points: Vec<Point> = (0..40_000)
        .map(|k| {
            let (s, c) = (k as f64 * std::f64::consts::TAU / 40_000.0).sin_cos();
            let r = if k % 2 == 0 { 10_000.0 } else { 9_999.0 };
            Point::new(20_000.0 + r * c, 20_000.0 + r * s)
        })
        .collect();
    let path = PathData::single(vectorcraft_geom::SubPath::polyline(&points, true));
    let n = Node::path(NodeId(1), path, Appearance::basic(Paint::solid(RED), Paint::solid(Color::BLACK), 2.0));
    for id in [EXTRUDE, INFLATE] {
        // Without caps: a cap of 40,000 corners takes more work to cut into triangles than an
        // export allows.
        let effect = fx(id, json!({ "cap": false }));
        let shapes = shapes_of(&[&n]);
        // The fill's mesh alone has more triangles than an export takes.
        let (mut once, mut apart) = (0, 0);
        let built = meshes_for(&effect, &shapes, &mut once, usize::MAX).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(built.len(), 1, "{id}: the fill's mesh alone");
        assert!(close(built[0].color, [0.8, 0.2, 0.1, 1.0]), "{id}");
        let reference = meshes_built_apart(&effect, &shapes, &mut apart, usize::MAX).unwrap();
        assert_eq!(mesh_bits(&built), mesh_bits(&reference), "{id}");
    }
}
