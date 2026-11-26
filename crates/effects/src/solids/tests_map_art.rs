use serde_json::json;
use vectorcraft_doc::Symbol;
use vectorcraft_geom::shapes;

use super::super::super::{CLASSIC_EXTRUDE, CLASSIC_ROTATE, MapSurface, map_surfaces_at, solid_art_in, validate_three_d_at_in, validate_three_d_in};
use super::*;
use crate::CLASSIC_REVOLVE;

const RED: Color = Color::Rgb { r: 1.0, g: 0.0, b: 0.0 };
const GREEN: Color = Color::Rgb { r: 0.0, g: 1.0, b: 0.0 };
const BLUE: Color = Color::Rgb { r: 0.0, g: 0.0, b: 1.0 };

fn square(id: u64, r: Rect, c: Color) -> Node {
    Node::path(NodeId(id), shapes::rectangle(r), Appearance::basic(Paint::solid(c), Paint::None, 0.0))
}

/// A document with symbol "Mark", a red 10 × 10 square, and symbol "Corner", 30 × 30: a red square
/// at its top left, a blue one to the right of it and a green one below it.
fn document() -> Document {
    let mut d = Document::new(1000.0, 1000.0);
    d.symbols.push(Symbol { dynamic: false, name: "Mark".into(), art: Arc::new(square(0, Rect::new(500.0, 500.0, 510.0, 510.0), RED)) });
    let corner = Node::group(
        NodeId(0),
        vec![
            Arc::new(square(0, Rect::new(0.0, 0.0, 10.0, 10.0), RED)),
            Arc::new(square(0, Rect::new(20.0, 0.0, 30.0, 10.0), BLUE)),
            Arc::new(square(0, Rect::new(0.0, 20.0, 10.0, 30.0), GREEN)),
        ],
    );
    d.symbols.push(Symbol { dynamic: false, name: "Corner".into(), art: Arc::new(corner) });
    d
}

/// A gray 100 × 100 square at (0, 0) with effect `id`, its parameters `params` over a view
/// straight on with No Shading.
fn object(id: &str, params: Value) -> Node {
    let mut p = json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "surface": "none"});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), params.as_object()) {
        p.extend(extra.clone());
    }
    let mut n = square(7, Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(0.5, 0.5, 0.5));
    n.appearance.effects.push(crate::new_effect(id, &p).unwrap());
    n
}

/// The paths of `art` filled with color `c`, in drawing order.
fn filled(art: &Node, c: Color) -> Vec<PathData> {
    let mut out = vec![];
    art.walk(&mut |n| {
        let fill = n.appearance.items.iter().find_map(|i| match i {
            AppearanceItem::Fill(f) => f.paint.color(),
            AppearanceItem::Stroke(_) => None,
        });
        if fill.is_some_and(|f| f.to_rgb().iter().zip(c.to_rgb()).all(|(a, b)| (a - b).abs() < 1e-4))
            && let Some(p) = n.path_data()
        {
            out.push(p.clone());
        }
    });
    out
}

fn bounds_of(paths: &[PathData]) -> Rect {
    paths.iter().filter_map(|p| p.bounds()).reduce(|a, b| a.union(b)).unwrap()
}

fn close(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() < 1e-6)
}

fn surfaces(n: &Node) -> Vec<MapSurface> {
    map_surfaces_at(Some(&document()), n, None, 0).unwrap()
}

/// Which way round the "Corner" symbol reads in `art`: positive when its blue square is to the
/// right of its red one and its green one below, as the symbol is drawn, at any turn; negative
/// when it is mirrored.
fn handedness(art: &Node) -> f64 {
    let at = |c: Color| bounds_of(&filled(art, c)).center();
    (at(BLUE) - at(RED)).cross(at(GREEN) - at(RED))
}

#[test]
fn the_surfaces_are_the_caps_then_each_wall_then_each_bevel() {
    let kinds = |n: &Node| surfaces(n).iter().map(|s| s.kind).collect::<Vec<_>>();
    let (front, back) = (SurfaceKind::End(PlanarFace::Front), SurfaceKind::End(PlanarFace::Back));
    let solid = object(CLASSIC_EXTRUDE, json!({}));
    assert_eq!(kinds(&solid), [vec![front, back], vec![SurfaceKind::Wall; 4]].concat());
    // Each wall runs from corner to corner of the square and is as deep as the solid.
    for wall in surfaces(&solid).iter().skip(2) {
        assert!((wall.width - 100.0).abs() < 1e-6 && (wall.height - 50.0).abs() < 1e-6, "{wall:?}");
    }
    assert_eq!(kinds(&object(CLASSIC_EXTRUDE, json!({"cap": false}))), vec![SurfaceKind::Wall; 4]);
    let beveled = object(CLASSIC_EXTRUDE, json!({"bevel": "classic"}));
    assert_eq!(kinds(&beveled), [vec![front, back], vec![SurfaceKind::Wall; 4], vec![SurfaceKind::Bevel; 4]].concat());
    // With no depth the art is a plane, which has its front only.
    assert_eq!(kinds(&object(CLASSIC_EXTRUDE, json!({"depth": 0}))), vec![front]);
    // An outline with no corner has one wall.
    let mut round = Node::path(NodeId(7), shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::solid(RED), Paint::None, 0.0));
    round.appearance.effects.push(crate::new_effect(CLASSIC_EXTRUDE, &json!({})).unwrap());
    assert_eq!(kinds(&round), vec![front, back, SurfaceKind::Wall]);
    // Revolve (Classic) has its side, and Rotate (Classic) takes no maps.
    assert_eq!(kinds(&object(CLASSIC_REVOLVE, json!({}))), vec![SurfaceKind::Side]);
    assert_eq!(map_surfaces_at(Some(&document()), &object(CLASSIC_ROTATE, json!({})), None, 0), None);
    // Seen straight on, only the front cap is turned toward the viewer.
    let seen: Vec<bool> = surfaces(&solid).iter().map(|s| s.visible).collect();
    assert_eq!(seen, [true, false, false, false, false, false]);
}

#[test]
fn a_bevel_as_deep_as_the_solid_leaves_no_walls_and_its_bevels_take_their_maps() {
    let (front, back) = (SurfaceKind::End(PlanarFace::Front), SurfaceKind::End(PlanarFace::Back));
    let solid = |maps: Value| object(CLASSIC_EXTRUDE, json!({"depth": 10, "bevel": "classic", "bevelHeight": 10, "mapArt": maps}));
    let kinds: Vec<SurfaceKind> = surfaces(&solid(json!([]))).iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [vec![front, back], vec![SurfaceKind::Bevel; 4]].concat());
    // Seen straight on, the front bevels draw the art mapped onto them.
    let bevels: Vec<Value> = (3..=6).map(|k| json!({"surface": k, "symbol": "Mark"})).collect();
    let art = solid_art_in(Some(&document()), &solid(json!(bevels))).unwrap();
    assert!(!filled(&art, RED).is_empty());
}

#[test]
fn a_map_on_the_front_cap_draws_the_symbol_at_its_offset_scales_and_turn() {
    let d = document();
    let map = |m: Value| solid_art_in(Some(&d), &object(CLASSIC_EXTRUDE, json!({"mapArt": [m]}))).unwrap();
    // Without a map nothing red is drawn.
    assert!(filled(&solid_art_in(Some(&d), &object(CLASSIC_EXTRUDE, json!({}))).unwrap(), RED).is_empty());
    // The art's center is at the center of the cap, its size the symbol's own.
    let centered = filled(&map(json!({"surface": 1, "symbol": "Mark"})), RED);
    assert!(close(bounds_of(&centered), Rect::new(45.0, 45.0, 55.0, 55.0)), "{:?}", bounds_of(&centered));
    // An offset moves it, and each scale sizes one side.
    let moved = filled(&map(json!({"surface": 1, "symbol": "Mark", "x": 10, "y": -5, "scaleX": 200, "scaleY": 300})), RED);
    assert!(close(bounds_of(&moved), Rect::new(50.0, 30.0, 70.0, 60.0)), "{:?}", bounds_of(&moved));
    // A quarter turn swaps the sides.
    let turned = filled(&map(json!({"surface": 1, "symbol": "Mark", "scaleX": 200, "rotation": 90})), RED);
    assert!(close(bounds_of(&turned), Rect::new(45.0, 40.0, 55.0, 60.0)), "{:?}", bounds_of(&turned));
    // The art is clipped to the cap.
    let past = filled(&map(json!({"surface": 1, "symbol": "Mark", "x": 50})), RED);
    assert_eq!(past.len(), 1);
    let art = map(json!({"surface": 1, "symbol": "Mark", "x": 50}));
    let mut clipped = false;
    art.walk(&mut |n| clipped |= matches!(&n.kind, NodeKind::Group { clip: true, .. }));
    assert!(clipped);
}

#[test]
fn a_map_on_a_side_wall_reads_the_right_way_round_from_outside() {
    let d = document();
    let all_walls: Vec<Value> = (3..=6).map(|k| json!({"surface": k, "symbol": "Corner"})).collect();
    for turn in [-90.0, 90.0] {
        // Seen from one side, one wall faces the viewer: it alone carries its map.
        let n = object(CLASSIC_EXTRUDE, json!({"rotationY": turn, "mapArt": all_walls}));
        let art = solid_art_in(Some(&d), &n).unwrap();
        assert_eq!(filled(&art, RED).len(), 1, "one wall is seen at {turn}");
        // The wall is 100 along the outline and 50 deep: the art keeps its size at its middle.
        let whole = bounds_of(&[filled(&art, RED), filled(&art, BLUE), filled(&art, GREEN)].concat());
        assert!((whole.width() - 30.0).abs() < 1e-6 && (whole.height() - 30.0).abs() < 1e-6, "{whole:?}");
        assert!((whole.center().x - 50.0).abs() < 1e-6 && (whole.center().y - 50.0).abs() < 1e-6, "{whole:?}");
        assert!(handedness(&art) > 0.0, "the art is mirrored at {turn}");
    }
    // The walls seen from above and below read the right way round too.
    for turn in [-90.0, 90.0] {
        let n = object(CLASSIC_EXTRUDE, json!({"rotationX": turn, "mapArt": all_walls}));
        assert!(handedness(&solid_art_in(Some(&d), &n).unwrap()) > 0.0, "the art is mirrored at {turn}");
    }
}

#[test]
fn a_wall_beside_a_bevel_is_laid_out_at_its_own_length() {
    let d = document();
    // A bevel 10 wide added around the square sets each wall 10 out, so each wall is 120 long; a
    // bevel takes the length of the wall it sits above. Carved in, the walls are the square's own.
    for (inside, length) in [(false, 120.0), (true, 100.0)] {
        let solid = object(CLASSIC_EXTRUDE, json!({"bevel": "classic", "bevelHeight": 10, "bevelInside": inside}));
        for side in surfaces(&solid).iter().skip(2) {
            assert!((side.width - length).abs() < 1e-6, "inside {inside}: {side:?}");
        }
        // Seen from one side, the art on the wall that faces the viewer keeps its size.
        let walls: Vec<Value> = (3..=6).map(|k| json!({"surface": k, "symbol": "Corner"})).collect();
        let n = object(CLASSIC_EXTRUDE, json!({"bevel": "classic", "bevelHeight": 10, "bevelInside": inside, "rotationY": 90, "mapArt": walls}));
        let art = solid_art_in(Some(&d), &n).unwrap();
        let whole = bounds_of(&[filled(&art, RED), filled(&art, BLUE), filled(&art, GREEN)].concat());
        assert!((whole.width() - 30.0).abs() < 1e-6 && (whole.height() - 30.0).abs() < 1e-6, "inside {inside}: {whole:?}");
    }
}

#[test]
fn art_on_a_facet_with_a_curved_edge_is_clipped_to_that_edge() {
    let quad = vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)];
    // The facet's top edge follows a curve that bows into the quad, down to y = 2.
    let mut curved = BezPath::new();
    curved.move_to((0.0, 0.0));
    curved.curve_to((10.0 / 3.0, 8.0 / 3.0), (20.0 / 3.0, 8.0 / 3.0), (10.0, 0.0));
    curved.line_to((10.0, 10.0));
    curved.line_to((0.0, 10.0));
    curved.close_path();
    let facet = Facet {
        contours: vec![quad.clone()],
        depth: 0.0,
        brightness: 1.0,
        illumination: [1.0; 3],
        normal: [0.0, 0.0, 1.0],
        rgb_gradient: None,
        outline: Some(PathData::from_bezpath(&curved)),
        side: true,
        away: false,
        end: None,
        plane: None,
        surface: None,
        membrane: None,
        cell: None,
    };
    let cell = MapCell { surface: 0, page: quad.clone(), uv: quad.clone() };
    let whole = Rect::new(0.0, 0.0, 10.0, 10.0);
    let art = Unrolled {
        polygons: vec![quad],
        boxes: vec![whole],
        bounds: whole,
        rule: FillRule::NonZero,
        paint: Paint::solid(RED),
        opacity: 1.0,
        blend: Default::default(),
        place: Affine::IDENTITY,
    };
    let (mut budget, mut copies) = (MapBudget::default(), 0);
    let out = cell_art(NodeId(1), &[art], &cell, &facet, &|c| c, &mut budget, &mut copies).unwrap();
    let area: f64 = out.iter().filter_map(|n| n.path_data()).map(|p| vectorcraft_geom::kurbo::Shape::area(&p.to_bezpath()).abs()).sum();
    let want = vectorcraft_geom::kurbo::Shape::area(&curved).abs();
    assert!((area - want).abs() < 1e-2, "{area} drawn, {want} inside the curved edge");
}

#[test]
fn art_on_a_facet_whose_curved_edge_bows_out_covers_the_whole_facet() {
    let quad = vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)];
    // The facet's top edge follows a curve that bows out of the quad, up to y = -2.
    let mut curved = BezPath::new();
    curved.move_to((0.0, 0.0));
    curved.curve_to((10.0 / 3.0, -8.0 / 3.0), (20.0 / 3.0, -8.0 / 3.0), (10.0, 0.0));
    curved.line_to((10.0, 10.0));
    curved.line_to((0.0, 10.0));
    curved.close_path();
    let facet = Facet {
        contours: vec![quad.clone()],
        depth: 0.0,
        brightness: 1.0,
        illumination: [1.0; 3],
        normal: [0.0, 0.0, 1.0],
        rgb_gradient: None,
        outline: Some(PathData::from_bezpath(&curved)),
        side: true,
        away: false,
        end: None,
        plane: None,
        surface: None,
        membrane: None,
        cell: None,
    };
    let cell = MapCell { surface: 0, page: quad.clone(), uv: quad };
    // Art that covers the facet and more.
    let cover = vec![Point::new(-5.0, -5.0), Point::new(15.0, -5.0), Point::new(15.0, 15.0), Point::new(-5.0, 15.0)];
    let whole = Rect::new(-5.0, -5.0, 15.0, 15.0);
    let art = Unrolled {
        polygons: vec![cover],
        boxes: vec![whole],
        bounds: whole,
        rule: FillRule::NonZero,
        paint: Paint::solid(RED),
        opacity: 1.0,
        blend: Default::default(),
        place: Affine::IDENTITY,
    };
    let (mut budget, mut copies) = (MapBudget::default(), 0);
    let out = cell_art(NodeId(1), &[art], &cell, &facet, &|c| c, &mut budget, &mut copies).unwrap();
    let area: f64 = out.iter().filter_map(|n| n.path_data()).map(|p| vectorcraft_geom::kurbo::Shape::area(&p.to_bezpath()).abs()).sum();
    let want = vectorcraft_geom::kurbo::Shape::area(&curved).abs();
    assert!((area - want).abs() < 1e-2, "{area} drawn, {want} inside the curved edge");
}

#[test]
fn the_bounds_tests_of_contours_away_from_a_cell_count_as_cuts() {
    let quad = vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)];
    let facet = Facet {
        contours: vec![quad.clone()],
        depth: 0.0,
        brightness: 1.0,
        illumination: [1.0; 3],
        normal: [0.0, 0.0, 1.0],
        rgb_gradient: None,
        outline: None,
        side: true,
        away: false,
        end: None,
        plane: None,
        surface: None,
        membrane: None,
        cell: None,
    };
    let cell = MapCell { surface: 0, page: quad.clone(), uv: quad };
    // Small contours on either side of the cell: the piece's bounds reach across it, but no
    // contour's own bounds do.
    let polygons: Vec<Vec<Point>> = (0..2000)
        .map(|k| {
            let x = if k % 2 == 0 { -1000.0 } else { 1000.0 };
            vec![Point::new(x, 0.0), Point::new(x + 1.0, 0.0), Point::new(x + 1.0, 1.0)]
        })
        .collect();
    let boxes: Vec<Rect> = polygons.iter().filter_map(|p| box_of(p)).collect();
    let art = Unrolled {
        polygons,
        boxes,
        bounds: Rect::new(-1000.0, 0.0, 1001.0, 1.0),
        rule: FillRule::NonZero,
        paint: Paint::solid(RED),
        opacity: 1.0,
        blend: Default::default(),
        place: Affine::IDENTITY,
    };
    let mut budget = MapBudget { cuts: MAX_SURFACE_CUTS - 10, ..MapBudget::default() };
    assert!(cell_art(NodeId(1), &[art], &cell, &facet, &|c| c, &mut budget, &mut 0).is_none());
}

#[test]
fn a_map_on_the_back_cap_reads_the_right_way_round_from_behind() {
    let d = document();
    let n = object(CLASSIC_EXTRUDE, json!({"rotationY": 180, "mapArt": [{"surface": 1, "symbol": "Mark"}, {"surface": 2, "symbol": "Corner"}]}));
    let art = solid_art_in(Some(&d), &n).unwrap();
    // Only the back cap is seen, and it carries its own map's art.
    assert_eq!(filled(&art, RED).len(), 1);
    assert_eq!((filled(&art, BLUE).len(), filled(&art, GREEN).len()), (1, 1));
    assert!(handedness(&art) > 0.0);
}

#[test]
fn art_past_the_shared_limits_leaves_out_the_mapped_art_of_every_solid() {
    let mut d = document();
    // A zigzag of half the straight sides one 3D effect's mapped art may draw.
    let mut b = BezPath::new();
    b.move_to((0.0, 0.0));
    for i in 1..=MAX_ELEMENTS / 2 {
        b.line_to((i as f64 * 0.00005, (i % 2) as f64));
    }
    b.close_path();
    let half = Node::path(NodeId(0), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::None, 0.0));
    d.symbols.push(Symbol { dynamic: false, name: "Half".into(), art: Arc::new(half) });
    let gray = Color::rgb(0.5, 0.5, 0.5);
    // A group of two squares: the first solid's front is surface 1, the second's surface 7.
    let art = |maps: Value, invisible: bool| {
        let members =
            vec![Arc::new(square(8, Rect::new(0.0, 0.0, 100.0, 100.0), gray)), Arc::new(square(9, Rect::new(200.0, 0.0, 300.0, 100.0), gray))];
        let mut g = Node::group(NodeId(7), members);
        let p = json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "surface": "none", "mapInvisible": invisible, "mapArt": maps});
        g.appearance.effects.push(crate::new_effect(CLASSIC_EXTRUDE, &p).unwrap());
        solid_art_in(Some(&d), &g).unwrap()
    };
    // One solid's art fits.
    assert!(!filled(&art(json!([{"surface": 1, "symbol": "Half"}]), false), RED).is_empty());
    // Both together are past the limit: no solid draws its art, and with Invisible Geometry every
    // solid's faces draw.
    let both = json!([{"surface": 1, "symbol": "Half"}, {"surface": 7, "symbol": "Half"}]);
    assert!(filled(&art(both.clone(), false), RED).is_empty());
    let hidden = art(both, true);
    assert!(filled(&hidden, RED).is_empty());
    let faces = bounds_of(&filled(&hidden, gray));
    assert!(faces.x0 < 1.0 && faces.x1 > 299.0, "{faces:?}");
}

#[test]
fn a_wall_under_a_perspective_maps_its_art_whole() {
    let d = document();
    let all_walls: Vec<Value> = (3..=6).map(|k| json!({"surface": k, "symbol": "Mark"})).collect();
    let n = object(CLASSIC_EXTRUDE, json!({"rotationY": -60, "perspectiveAngle": 90, "mapArt": all_walls}));
    let art = solid_art_in(Some(&d), &n).unwrap();
    // The wall is one flat face: the square maps onto it as one four-cornered area, not as two
    // pieces that meet along the face's diagonal.
    let marks = filled(&art, RED);
    assert_eq!(marks.len(), 1);
    let mark = &marks[0];
    assert_eq!((mark.subpaths.len(), mark.subpaths[0].anchors.len()), (1, 4), "{mark:?}");
    // Nearer the viewer the square's side is longer.
    let side = |i: usize| mark.subpaths[0].anchors[i].p.distance(mark.subpaths[0].anchors[(i + 1) % 4].p);
    let uneven = (side(0) - side(2)).abs().max((side(1) - side(3)).abs());
    assert!(uneven > 0.05, "{} {} {} {}", side(0), side(1), side(2), side(3));
}

#[test]
fn shade_artwork_and_invisible_geometry_change_what_is_drawn() {
    let d = document();
    let params = |extra: Value| {
        let mut p = json!({"surface": "diffuse", "rotationY": -60, "lightIntensity": 0, "ambient": 20, "mapArt": [{"surface": 1, "symbol": "Mark"}]});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        solid_art_in(Some(&d), &object(CLASSIC_EXTRUDE, p)).unwrap()
    };
    // Unshaded, the art keeps its own color on a shaded face.
    assert_eq!(filled(&params(json!({})), RED).len(), 1);
    // Shaded, it darkens toward black with its face.
    let shaded = params(json!({"mapShade": true}));
    assert!(filled(&shaded, RED).is_empty());
    let mut darkest = 1.0f32;
    shaded.walk(&mut |n| {
        for item in &n.appearance.items {
            if let AppearanceItem::Fill(f) = item
                && let Some([r, g, b]) = f.paint.color().map(|c| c.to_rgb())
                && g < 1e-4
                && b < 1e-4
                && r > 0.0
            {
                darkest = darkest.min(r);
            }
        }
    });
    assert!(darkest < 0.999, "the art is as dark as its face: {darkest}");
    // With Invisible Geometry only the art is drawn.
    let count = |art: &Node| {
        let mut paths = 0;
        art.walk(&mut |n| paths += usize::from(n.path_data().is_some() && !n.appearance.items.is_empty()));
        paths
    };
    assert!(count(&params(json!({}))) > 1);
    assert_eq!(count(&params(json!({"mapInvisible": true}))), 1);
    // Without a map Invisible Geometry leaves the solid as it is, and Wireframe draws no art.
    let plain = object(CLASSIC_EXTRUDE, json!({"mapInvisible": true}));
    assert!(count(&solid_art_in(Some(&d), &plain).unwrap()) > 0);
    assert!(filled(&params(json!({"surface": "wireframe"})), RED).is_empty());
}

#[test]
fn a_map_past_the_surfaces_draws_nothing_and_a_missing_symbol_is_left_out() {
    let d = document();
    let art = |maps: Value| solid_art_in(Some(&d), &object(CLASSIC_EXTRUDE, json!({"mapArt": maps}))).unwrap();
    assert!(filled(&art(json!([{"surface": 7, "symbol": "Mark"}])), RED).is_empty());
    assert!(filled(&art(json!([{"surface": 1, "symbol": "Gone"}])), RED).is_empty());
    // A hollow solid's first surface is its first wall, so the cap's map draws nothing there.
    let hollow = object(CLASSIC_EXTRUDE, json!({"cap": false, "mapArt": [{"surface": 1, "symbol": "Mark"}]}));
    assert!(filled(&solid_art_in(Some(&d), &hollow).unwrap(), RED).is_empty());
    assert_eq!(missing(&d, &json!({"mapArt": [{"surface": 1, "symbol": "Gone"}]})), Some("Gone".to_string()));
    assert_eq!(missing(&d, &json!({"mapArt": [{"surface": 1, "symbol": "Mark"}]})), None);
}

#[test]
fn the_points_drawn_on_earlier_surfaces_leave_the_next_symbol_its_own_allowance() {
    let d = document();
    let art = MapArt::of(Some(&d), CLASSIC_EXTRUDE, &json!({"mapArt": [{"surface": 3, "symbol": "Mark"}]})).unwrap();
    let wall = SolidSurface { kind: SurfaceKind::Wall, width: 100.0, height: 50.0, center: None };
    // Earlier surfaces drew more points on their faces than one symbol may take cut into pieces.
    let mut budget = MapBudget { points: MAX_ELEMENTS + 1, ..MapBudget::default() };
    let pieces = art.unrolled(&art.maps[0], &wall, &mut budget).expect("the symbol is cut into pieces");
    assert_eq!(pieces.len(), 1);
}

#[test]
fn art_past_the_paint_copies_allowance_leaves_the_allowance_to_later_art() {
    let d = document();
    let mut b = BezPath::new();
    b.move_to((100.0, 0.0));
    b.line_to((100.0, 100.0));
    let path = PathData::from_bezpath(&b);
    let gray = Paint::solid(Color::rgb(0.5, 0.5, 0.5));
    let n = Node::path(NodeId(7), path.clone(), Appearance::basic(Paint::None, gray.clone(), 1.0));
    let p = crate::merged_params(
        CLASSIC_REVOLVE,
        &json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "surface": "none", "offset": 40, "mapArt": [{"surface": 1, "symbol": "Mark"}]}),
    );
    let art = MapArt::of(Some(&d), CLASSIC_REVOLVE, &p).unwrap();
    let bounds = path.bounds().unwrap();
    let options = vectorcraft_three_d::Revolve { map_surface: true, ..crate::revolve::options_of(CLASSIC_REVOLVE, &p) };
    let (width, height) = vectorcraft_three_d::revolve_surface_size(&path, bounds, options).unwrap();
    let layout = SolidLayout { first: 0, surfaces: vec![SolidSurface { kind: SurfaceKind::Side, width, height, center: None }], ends: None };
    let facets = vectorcraft_three_d::revolve_facets(&path, bounds, options).unwrap();
    // The side's art, with the copies of its paint and its points counted from `copies` and
    // `budget` already used.
    let draw = |copies: &mut usize, budget: &mut MapBudget| {
        let mapped = crate::solids::classic::Mapped { art: &art, layout: &layout, budget, copies };
        let blend = vectorcraft_doc::color::BlendMode::Normal;
        crate::solids::classic::surface(&n, &facets, &gray, (1.0, blend), &p, Some(mapped), false).0
    };
    let (mut copies, mut budget) = (0, MapBudget::default());
    assert!(!filled(&draw(&mut copies, &mut budget), RED).is_empty());
    assert!(copies > 0 && budget != MapBudget::default());
    // With the allowance used up, the art is left out, and so are the copies and the points it
    // would have taken.
    let full = crate::revolve::MAX_PAINT_COPIES;
    let (mut copies, mut budget) = (full, MapBudget::default());
    assert!(filled(&draw(&mut copies, &mut budget), RED).is_empty());
    assert_eq!(copies, full);
    assert_eq!(budget, MapBudget::default());
}

fn missing(d: &Document, p: &Value) -> Option<String> {
    super::super::missing_symbol(d, p, None)
}

#[test]
fn revolve_classic_wraps_its_map_around_the_side() {
    let d = document();
    let mut b = BezPath::new();
    b.move_to((100.0, 0.0));
    b.line_to((100.0, 100.0));
    let mut n = Node::path(NodeId(7), PathData::from_bezpath(&b), Appearance::basic(Paint::None, Paint::solid(Color::rgb(0.5, 0.5, 0.5)), 1.0));
    let view =
        json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "surface": "none", "offset": 40, "mapArt": [{"surface": 1, "symbol": "Corner"}]});
    n.appearance.effects.push(crate::new_effect(CLASSIC_REVOLVE, &view).unwrap());
    let art = crate::revolve_art_in(Some(&d), &n).unwrap();
    // The art is drawn at the middle of the side that faces the viewer, the right way round.
    assert!(!filled(&art, RED).is_empty());
    assert!(handedness(&art) > 0.0);
    let whole = bounds_of(&[filled(&art, RED), filled(&art, BLUE), filled(&art, GREEN)].concat());
    assert!((whole.center().y - 50.0).abs() < 1.0, "{whole:?}");
    // A map of another surface draws nothing.
    n.appearance.effects[0].params["mapArt"] = json!([{"surface": 2, "symbol": "Corner"}]);
    assert!(filled(&crate::revolve_art_in(Some(&d), &n).unwrap(), RED).is_empty());
}

#[test]
fn invisible_geometry_leaves_out_every_member_of_a_revolved_group() {
    let d = document();
    let gray = Color::rgb(0.5, 0.5, 0.5);
    let line = |id: u64, x: f64, height: f64| {
        let mut b = BezPath::new();
        b.move_to((x, 0.0));
        b.line_to((x, height));
        Arc::new(Node::path(NodeId(id), PathData::from_bezpath(&b), Appearance::basic(Paint::None, Paint::solid(gray), 1.0)))
    };
    // The map goes on the side of the longer member; the shorter one has none.
    let art = |extra: Value| {
        let mut p =
            json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "surface": "none", "offset": 40, "mapArt": [{"surface": 1, "symbol": "Mark"}]});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let mut g = Node::group(NodeId(7), vec![line(8, 100.0, 100.0), line(9, 140.0, 30.0)]);
        g.appearance.effects.push(crate::new_effect(CLASSIC_REVOLVE, &p).unwrap());
        crate::revolve_art_in(Some(&d), &g).unwrap()
    };
    let shown = art(json!({}));
    assert!(!filled(&shown, RED).is_empty() && !filled(&shown, gray).is_empty());
    let hidden = art(json!({"mapInvisible": true}));
    assert!(!filled(&hidden, RED).is_empty());
    assert!(filled(&hidden, gray).is_empty(), "no member's faces are drawn");
    // On Wireframe the members are drawn as they are without a map.
    let wire = art(json!({"mapInvisible": true, "surface": "wireframe"}));
    let mut strokes = 0;
    wire.walk(&mut |n| strokes += usize::from(n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(_)))));
    assert!(strokes > 0);
}

#[test]
fn a_revolved_group_past_its_shared_limits_lists_no_surface() {
    let gray = Color::rgb(0.5, 0.5, 0.5);
    // Profiles of five anchors each; one alone fits Revolve's limits.
    let profile = |id: u64, x: f64| {
        let mut b = BezPath::new();
        b.move_to((x, 0.0));
        for i in 1..5 {
            b.line_to((x + f64::from(i % 2), f64::from(i) * 10.0));
        }
        Arc::new(Node::path(NodeId(id), PathData::from_bezpath(&b), Appearance::basic(Paint::None, Paint::solid(gray), 1.0)))
    };
    let group = |count: u64| {
        let mut g = Node::group(NodeId(7), (0..count).map(|k| profile(k + 8, 100.0 + k as f64 * 0.01)).collect());
        g.appearance.effects.push(crate::new_effect(CLASSIC_REVOLVE, &json!({})).unwrap());
        g
    };
    let d = document();
    assert_eq!(map_surfaces_at(Some(&d), &group(2), None, 0).map(|s| s.len()), Some(1));
    // 4,000 of them together are past the anchors the members share.
    assert!(map_surfaces_at(Some(&d), &group(4000), None, 0).is_none());
}

#[test]
fn an_extruded_group_past_the_faces_of_one_object_lists_no_surface() {
    // Polygons of 1,000 corners: each solid alone has about 1,000 faces.
    let polygon = |id: u64, x: f64| {
        let mut b = BezPath::new();
        for i in 0..1000 {
            let a = f64::from(i) * std::f64::consts::TAU / 1000.0;
            let p = (x + 40.0 * a.cos(), 40.0 * a.sin());
            if i == 0 { b.move_to(p) } else { b.line_to(p) }
        }
        b.close_path();
        Arc::new(Node::path(NodeId(id), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::None, 0.0)))
    };
    let group = |count: u64| {
        let mut g = Node::group(NodeId(7), (0..count).map(|k| polygon(k + 8, k as f64 * 100.0)).collect());
        g.appearance.effects.push(crate::new_effect(CLASSIC_EXTRUDE, &json!({"drawHiddenFaces": true})).unwrap());
        g
    };
    let d = document();
    assert!(map_surfaces_at(Some(&d), &group(2), None, 0).is_some());
    // 200 of them together have more faces than one object draws.
    assert!(map_surfaces_at(Some(&d), &group(200), None, 0).is_none());
}

#[test]
fn bad_maps_are_refused() {
    let d = document();
    let check = |id: &str, p: Value| {
        let n = object(id, json!({}));
        validate_three_d_in(Some(&d), &n, id, &crate::merged_params(id, &p))
    };
    assert_eq!(check(CLASSIC_EXTRUDE, json!({"mapArt": [{"surface": 3, "symbol": "Mark", "scaleX": 50}]})), Ok(()));
    assert_eq!(check(CLASSIC_REVOLVE, json!({"mapArt": [{"surface": 1, "symbol": "Mark"}], "mapShade": true})), Ok(()));
    for (p, error) in [
        (json!({"mapArt": {}}), "`mapArt` must be an array"),
        (json!({"mapArt": [7]}), "each of `mapArt` must be an object"),
        (json!({"mapArt": [{"symbol": "Mark"}]}), "needs `surface`"),
        (json!({"mapArt": [{"surface": 0, "symbol": "Mark"}]}), "needs `surface`"),
        (json!({"mapArt": [{"surface": 1.5, "symbol": "Mark"}]}), "needs `surface`"),
        (json!({"mapArt": [{"surface": 100001, "symbol": "Mark"}]}), "needs `surface`"),
        (json!({"mapArt": [{"surface": 1}]}), "needs `symbol`"),
        (json!({"mapArt": [{"surface": 1, "symbol": "Mark", "face": "front"}]}), "not `face`"),
        (json!({"mapArt": [{"surface": 1, "symbol": "Mark", "scaleX": 0}]}), "scaleX must be a number from 1 to 10000"),
        (json!({"mapArt": [{"surface": 1, "symbol": "Mark", "rotation": 361}]}), "rotation must be a number from -360 to 360"),
        (json!({"mapArt": [{"surface": 1, "symbol": "Mark", "x": "left"}]}), "x must be a number"),
        (json!({"mapArt": [{"surface": 1, "symbol": "Mark"}, {"surface": 1, "symbol": "Corner"}]}), "maps surface 1 more than once"),
        (json!({"mapShade": 1}), "`mapShade` must be true or false"),
        (json!({"mapInvisible": "yes"}), "`mapInvisible` must be true or false"),
    ] {
        for id in [CLASSIC_EXTRUDE, CLASSIC_REVOLVE] {
            let e = check(id, p.clone()).unwrap_err();
            assert!(e.contains(error), "{id} {p}: {e}");
        }
    }
    let many: Vec<Value> = (1..=65).map(|k| json!({"surface": k, "symbol": "Mark"})).collect();
    assert!(check(CLASSIC_EXTRUDE, json!({"mapArt": many})).unwrap_err().contains("at most 64 maps"));
    // Rotate (Classic) has no surface to map onto.
    let e = check(CLASSIC_ROTATE, json!({"mapArt": [{"surface": 1, "symbol": "Mark"}]})).unwrap_err();
    assert!(e.contains(MAP_ART_NOT_TAKEN), "{e}");
}

#[test]
fn a_mapped_symbol_past_the_limits_of_a_graphics_art_is_refused() {
    let mut d = document();
    // A zigzag of more straight sides than one 3D effect's graphics may draw.
    let mut b = BezPath::new();
    b.move_to((0.0, 0.0));
    for i in 1..=MAX_ELEMENTS {
        b.line_to((i as f64 * 0.001, (i % 2) as f64));
    }
    b.close_path();
    let dense = Node::path(NodeId(0), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::None, 0.0));
    d.symbols.push(Symbol { dynamic: false, name: "Dense".into(), art: Arc::new(dense) });
    for id in [CLASSIC_EXTRUDE, CLASSIC_REVOLVE] {
        let p = crate::merged_params(id, &json!({"mapArt": [{"surface": 1, "symbol": "Dense"}]}));
        let e = validate_three_d_in(Some(&d), &object(id, json!({})), id, &p).unwrap_err();
        assert!(e.contains("too complex"), "{id}: {e}");
    }
    // On a fill, as on the object.
    let plain = square(7, Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(0.5, 0.5, 0.5));
    let p = crate::merged_params(CLASSIC_REVOLVE, &json!({"mapArt": [{"surface": 1, "symbol": "Dense"}]}));
    let e = validate_three_d_at_in(Some(&d), &plain, CLASSIC_REVOLVE, &p, (Some(0), 0)).unwrap_err();
    assert!(e.contains("too complex"), "{e}");
}

#[test]
fn a_mapped_symbol_whose_curves_cut_into_too_many_pieces_is_refused() {
    let mut d = document();
    // Large circles: a few curves each, but many straight pieces once they are cut.
    let loops = |name: &str, count: usize| {
        let mut b = BezPath::new();
        for k in 0..count {
            let r = 1e5;
            let circle = vectorcraft_geom::kurbo::Circle::new((k as f64 * 2.1 * r, 0.0), r);
            b.extend(vectorcraft_geom::kurbo::Shape::path_elements(&circle, 0.1));
        }
        let art = Node::path(NodeId(0), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::None, 0.0));
        Symbol { dynamic: false, name: name.into(), art: Arc::new(art) }
    };
    d.symbols.push(loops("Loop", 1));
    d.symbols.push(loops("Loops", 400));
    let check = |id: &str, surface: usize, symbol: &str| {
        let p = crate::merged_params(id, &json!({"mapArt": [{"surface": surface, "symbol": symbol}]}));
        validate_three_d_in(Some(&d), &object(id, json!({})), id, &p)
    };
    // Extrude & Bevel's surface 3 is a wall, and Revolve's surface 1 its side.
    for (id, surface) in [(CLASSIC_EXTRUDE, 3), (CLASSIC_REVOLVE, 1)] {
        assert_eq!(check(id, surface, "Loop"), Ok(()), "{id}");
        let e = check(id, surface, "Loops").unwrap_err();
        assert!(e.contains("too complex"), "{id}: {e}");
    }
    // A cap draws the curves as they are, without cutting them.
    assert_eq!(check(CLASSIC_EXTRUDE, 1, "Loops"), Ok(()));
}

#[test]
fn maps_are_set_cleared_and_renamed_with_their_symbols() {
    let p = json!({"depth": 20});
    let map =
        |surface: usize, symbol: &str| SurfaceMap { surface, symbol: symbol.into(), x: 0.0, y: 0.0, scale_x: 100.0, scale_y: 100.0, rotation: 0.0 };
    let p = with_surface_map(&p, 4, Some(&map(4, "Mark")));
    let p = with_surface_map(&p, 2, Some(&map(2, "Corner")));
    let p = with_surface_map(&p, 4, Some(&map(4, "Corner")));
    // One map for a surface, in the order of the surfaces; the other parameters stay.
    assert_eq!(surface_maps(&p), vec![map(2, "Corner"), map(4, "Corner")]);
    assert_eq!(p["depth"], json!(20));
    assert_eq!(surface_maps(&with_surface_map(&p, 2, None)), vec![map(4, "Corner")]);
    // A file's maps: entries that are not valid and a surface's second map are left out.
    let loaded = json!({"mapArt": [{"surface": 1, "symbol": "Mark"}, {"surface": 1, "symbol": "Corner"}, {"surface": 0, "symbol": "Mark"}, 3]});
    assert_eq!(surface_maps(&loaded), vec![map(1, "Mark")]);
    // Renaming a symbol renames it in the maps of the effects that take them.
    let mut n = object(CLASSIC_EXTRUDE, json!({"mapArt": [{"surface": 1, "symbol": "Mark"}]}));
    assert_eq!(super::super::graphic_symbols(&n), vec!["Mark".to_string()]);
    assert!(super::super::rename_graphic_symbol(&mut n, "Mark", "Logo"));
    assert_eq!(surface_maps(&n.appearance.effects[0].params)[0].symbol, "Logo");
    // Scale to Fit fills the surface: each side by its own scale when the art is square to it.
    assert_eq!(map(1, "Mark").fitting_scales(Rect::new(0.0, 0.0, 10.0, 10.0), (100.0, 50.0)), Some((1000.0, 500.0)));
    let turned = SurfaceMap { rotation: 90.0, ..map(1, "Mark") };
    assert_eq!(turned.fitting_scales(Rect::new(0.0, 0.0, 10.0, 20.0), (100.0, 50.0)), Some((500.0, 500.0)));
    let aslant = SurfaceMap { rotation: 45.0, ..map(1, "Mark") };
    let (sx, sy) = aslant.fitting_scales(Rect::new(0.0, 0.0, 10.0, 10.0), (100.0, 50.0)).unwrap();
    assert!((sx - sy).abs() < 1e-9 && (sx - 50.0 / (20.0 * 0.5f64.sqrt()) * 100.0).abs() < 1e-6, "{sx} {sy}");
    assert_eq!(map(1, "Mark").fitting_scales(Rect::new(0.0, 0.0, 0.0, 10.0), (100.0, 50.0)), None);
}
