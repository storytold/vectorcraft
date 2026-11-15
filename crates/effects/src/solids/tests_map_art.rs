use serde_json::json;
use vectorcraft_doc::Symbol;
use vectorcraft_geom::shapes;

use super::super::super::{CLASSIC_EXTRUDE, CLASSIC_ROTATE, MapSurface, map_surfaces_at, solid_art_in, validate_three_d_in};
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
fn a_map_on_the_back_cap_reads_the_right_way_round_from_behind() {
    let d = document();
    let n = object(CLASSIC_EXTRUDE, json!({"rotationY": 180, "mapArt": [{"surface": 1, "symbol": "Corner"}, {"surface": 2, "symbol": "Corner"}]}));
    let art = solid_art_in(Some(&d), &n).unwrap();
    // Only the back cap is seen.
    assert_eq!(filled(&art, RED).len(), 1);
    assert!(handedness(&art) > 0.0);
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
