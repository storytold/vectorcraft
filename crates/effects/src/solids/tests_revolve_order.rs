use serde_json::json;

use super::*;
use crate::revolve::{CLASSIC_REVOLVE, REVOLVE, revolve_art, revolve_options};

/// The first pair of faces `(polygon, plane)`, in the order given, drawn the wrong way round where
/// their polygons overlap: the earlier one nearer the viewer at the middle of the overlap.
fn misordered(faces: &[(Vec<Point>, Option<DepthPlane>)]) -> Option<(usize, usize)> {
    for (i, (a, pa)) in faces.iter().enumerate() {
        let Some(pa) = pa.filter(|_| convex(a).is_some()) else { continue };
        for (j, (b, pb)) in faces.iter().enumerate().skip(i + 1) {
            let Some(pb) = pb else { continue };
            let Some(turn) = convex(b) else { continue };
            if !rects_overlap(polygon_bounds(a), polygon_bounds(b)) {
                continue;
            }
            let Some(mid) = centroid(&clip_convex(a, b, turn)) else { continue };
            let (Some(da), Some(db)) = (pa.depth_at(mid), pb.depth_at(mid)) else { continue };
            if da - db > 1e-6 * (1.0 + da.abs().max(db.abs())) {
                return Some((i, j));
            }
        }
    }
    None
}

/// An open, vase-like profile of straight edges.
fn vase() -> PathData {
    let mut b = BezPath::new();
    b.move_to((30.0, 20.0));
    for p in [(90.0, 20.0), (55.0, 65.0), (95.0, 110.0), (30.0, 145.0)] {
        b.line_to(p);
    }
    PathData::from_bezpath(&b)
}

/// The `k`th of five closed 14 × 20 pt rectangles stepping down and away from the axis, as the
/// dashes of a dashed line's outline do.
fn dash(k: usize) -> PathData {
    let (x, y) = (30.0 + 12.0 * k as f64, 20.0 + 26.0 * k as f64);
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.line_to((x + 14.0, y));
    b.line_to((x + 14.0, y + 20.0));
    b.line_to((x, y + 20.0));
    b.close_path();
    PathData::from_bezpath(&b)
}

/// The five [`dash`]es as one compound path.
fn dashes() -> PathData {
    PathData::new((0..5).flat_map(|k| dash(k).subpaths).collect())
}

/// A tilted three-quarter turn of [`vase`], whose faces sorted by their middles' depths draw a
/// nearer face under a farther one where the two overlap.
fn params() -> Value {
    json!({"angle": 270, "offset": 12, "rotationX": -30, "rotationY": 0, "perspectiveAngle": 0, "segments": 24, "cap": true})
}

/// A three-quarter turn of [`dashes`], steeply tilted and in wide perspective, whose classic facets
/// sorted by their middles' depths draw a nearer facet under a farther one where the two overlap.
fn classic_params() -> Value {
    json!({"angle": 270, "offset": 12, "rotationX": -60, "rotationY": 0, "perspectiveAngle": 80, "cap": true})
}

fn node(path: PathData) -> Node {
    Node::path(NodeId(1), path, Appearance::basic(Paint::solid(Color::rgb(0.1, 0.7, 0.8)), Paint::None, 0.0))
}

/// The faces of `faces` (polygon, plane and outline) in the order `art` draws their outlines.
fn drawn_order(art: &Node, faces: &[(Vec<Point>, Option<DepthPlane>, PathData)]) -> Vec<(Vec<Point>, Option<DepthPlane>)> {
    let mut out = vec![];
    art.walk(&mut |n| {
        if let Some(path) = n.path_data()
            && let Some((points, plane, _)) = faces.iter().find(|(.., outline)| outline == path)
        {
            out.push((points.clone(), *plane));
        }
    });
    out
}

/// Of a tilted, revolved vase, a face sorted by the depth of its middle goes under a farther face
/// that it is in front of where the two overlap. The surface draws the nearer face last there, on
/// its own and as a member of a revolved group.
#[test]
fn a_revolved_surface_draws_the_nearer_of_two_overlapping_faces_last() {
    let path = vase();
    let faces = vectorcraft_three_d::revolve(&path, path.bounds().unwrap(), revolve_options(&params())).unwrap();
    let by_depth: Vec<_> = faces.iter().map(|f| (f.points.clone(), f.plane)).collect();
    assert!(misordered(&by_depth).is_some(), "the faces by depth are drawn the wrong way round somewhere");
    let keyed: Vec<_> = faces.iter().map(|f| (f.points.clone(), f.plane, f.path())).collect();
    let mut n = node(path.clone());
    n.appearance.effects.push(crate::new_effect(REVOLVE, &params()).unwrap());
    let drawn = drawn_order(&revolve_art(&n).unwrap(), &keyed);
    assert_eq!(drawn.len(), faces.len());
    assert_eq!(misordered(&drawn), None, "an object");
    let mut g = Node::group(NodeId(9), vec![Arc::new(node(path))]);
    g.appearance.effects.push(crate::new_effect(REVOLVE, &params()).unwrap());
    let drawn = drawn_order(&revolve_art(&g).unwrap(), &keyed);
    assert_eq!(drawn.len(), faces.len());
    assert_eq!(misordered(&drawn), None, "a group's member");
}

/// The faces of a group's members, sorted together by the depths of their middles, go in the order
/// of their depths where they overlap, those of two members as well as those of one.
#[test]
fn a_revolved_groups_members_draw_the_nearer_of_two_overlapping_faces_last() {
    let p = json!({"angle": 270, "offset": 12, "rotationX": -30, "rotationY": 0, "perspectiveAngle": 0, "segments": 16, "cap": true});
    let members: Vec<PathData> = (0..5).map(dash).collect();
    let b = dashes().bounds().unwrap();
    let mut faces: Vec<_> = members.iter().flat_map(|m| vectorcraft_three_d::revolve(m, b, revolve_options(&p)).unwrap()).collect();
    faces.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    let by_depth: Vec<_> = faces.iter().map(|f| (f.points.clone(), f.plane)).collect();
    assert!(misordered(&by_depth).is_some(), "the faces by depth are drawn the wrong way round somewhere");
    let keyed: Vec<_> = faces.iter().map(|f| (f.points.clone(), f.plane, f.path())).collect();
    let mut g = Node::group(NodeId(9), members.into_iter().map(|m| Arc::new(node(m))).collect());
    g.appearance.effects.push(crate::new_effect(REVOLVE, &p).unwrap());
    let drawn = drawn_order(&revolve_art(&g).unwrap(), &keyed);
    assert_eq!(drawn.len(), faces.len());
    assert_eq!(misordered(&drawn), None);
}

/// Revolve (Classic) draws the nearer of two overlapping facets last where they overlap, too.
#[test]
fn a_classic_revolved_surface_draws_the_nearer_of_two_overlapping_facets_last() {
    let path = dashes();
    // Revolve (Classic) sweeps its surface unshaded, with one facet for each cell.
    let classic = vectorcraft_three_d::Revolve {
        shade: false,
        smooth_shading: false,
        lights: None,
        light_color: [1.0; 3],
        ..revolve_options(&crate::merged_params(CLASSIC_REVOLVE, &classic_params()))
    };
    let facets = vectorcraft_three_d::revolve_facets(&path, path.bounds().unwrap(), classic).unwrap();
    let by_depth: Vec<_> = facets.iter().map(|f| (f.contours[0].clone(), f.plane)).collect();
    assert!(misordered(&by_depth).is_some(), "the facets by depth are drawn the wrong way round somewhere");
    let keyed: Vec<_> = facets.iter().map(|f| (f.contours[0].clone(), f.plane, f.path())).collect();
    let mut n = node(path);
    n.appearance.effects.push(crate::new_effect(CLASSIC_REVOLVE, &classic_params()).unwrap());
    let drawn = drawn_order(&revolve_art(&n).unwrap(), &keyed);
    assert_eq!(drawn.len(), facets.len());
    assert_eq!(misordered(&drawn), None);
}

/// A surface whose faces by depth are already drawn the right way round wherever they overlap keeps
/// that order.
#[test]
fn a_surface_already_in_order_keeps_it() {
    let path = vase();
    let p = json!({"angle": 360, "offset": 12, "rotationX": 0, "rotationY": 0, "segments": 24, "cap": true});
    let faces = vectorcraft_three_d::revolve(&path, path.bounds().unwrap(), revolve_options(&p)).unwrap();
    let by_depth: Vec<_> = faces.iter().map(|f| (f.points.clone(), f.plane)).collect();
    assert_eq!(misordered(&by_depth), None);
    let order = surface_order(faces.iter().map(|f| (f.points.as_slice(), f.plane, f.depth)));
    assert_eq!(order, (0..faces.len()).collect::<Vec<_>>());
}

/// Of two overlapping faces, the one nearer the viewer where they overlap goes after the other,
/// whatever their depths; a face without a plane, and one that overlaps neither, keep their places.
/// Two faces at the same depth there keep their order.
#[test]
fn surface_order_puts_the_nearer_face_where_they_overlap_last() {
    let square = |x: f64| vec![Point::new(x, 0.0), Point::new(x + 10.0, 0.0), Point::new(x + 10.0, 10.0), Point::new(x, 10.0)];
    // z = 10 - x: at the middle of its overlap with the next square (x 7.5) it is at z 2.5.
    let r = std::f64::consts::FRAC_1_SQRT_2;
    let sloped = Some(DepthPlane { normal: [r, 0.0, r], offset: 10.0 * r, center: Point::ZERO, eye: None });
    let flat = |z: f64| Some(DepthPlane { normal: [0.0, 0.0, 1.0], offset: z, center: Point::ZERO, eye: None });
    let (a, b, apart) = (square(0.0), square(5.0), square(40.0));
    let faces = [(&a[..], sloped, 5.0), (&apart[..], flat(0.0), 1.0), (&b[..], flat(3.0), 3.0)];
    assert_eq!(surface_order(faces), vec![0, 1, 2], "the flat face, nearer at the overlap, is already last");
    let faces = [(&b[..], flat(2.0), 2.0), (&apart[..], flat(0.0), 3.0), (&a[..], sloped, 5.0)];
    assert_eq!(surface_order(faces), vec![0, 1, 2], "the sloped face, nearer at the overlap, is already last");
    // The sloped face's middle (x 5) is at z 5, but the flat face at z 3 is nearer at the overlap.
    let faces = [(&b[..], flat(3.0), 3.0), (&apart[..], flat(0.0), 4.0), (&a[..], sloped, 5.0)];
    assert_eq!(surface_order(faces), vec![1, 2, 0], "the flat face is nearer at the overlap");
    let faces = [(&b[..], None, 3.0), (&apart[..], flat(0.0), 4.0), (&a[..], sloped, 5.0)];
    assert_eq!(surface_order(faces), vec![0, 1, 2], "no plane");
    let faces = [(&b[..], flat(2.5), 3.0), (&a[..], sloped, 5.0)];
    assert_eq!(surface_order(faces), vec![0, 1], "the same depth there");
}
