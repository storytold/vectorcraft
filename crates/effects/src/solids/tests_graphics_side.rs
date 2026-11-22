//! Graphics on Extrude & Bevel's side: its walls and bevels unrolled flat.

use serde_json::json;
use vectorcraft_doc::color::Color;
use vectorcraft_doc::{StrokeLayer, Symbol};
use vectorcraft_geom::shapes;

use super::super::{EXTRUDE, graphic_frame, graphic_side_size, solid_art_in, validate_three_d_in};
use super::*;

fn red() -> Color {
    Color::rgb(1.0, 0.0, 0.0)
}

fn square(id: u64, r: Rect, paint: Paint) -> Node {
    Node::path(NodeId(id), shapes::rectangle(r), Appearance::basic(paint, Paint::None, 0.0))
}

/// A document with symbol "Mark": a red 10 × 10 square.
fn document() -> Document {
    let mut d = Document::new(1000.0, 1000.0);
    d.symbols.push(Symbol {
        dynamic: false,
        name: "Mark".into(),
        art: Arc::new(square(0, Rect::new(500.0, 500.0, 510.0, 510.0), Paint::solid(red()))),
    });
    d
}

/// A gray 100 × 100 square at (0, 0), extruded 50 pt, seen from above (its top wall faces the
/// viewer, its front edge at the bottom of the page), with `params` over that.
fn object(params: Value) -> Node {
    let mut p = json!({"rotationX": -90.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": false});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), params.as_object()) {
        p.extend(extra.clone());
    }
    let mut n = square(7, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(Color::rgb(0.5, 0.5, 0.5)));
    n.appearance.effects.push(crate::new_effect(EXTRUDE, &p).unwrap());
    n
}

fn is_red(p: &Paint) -> bool {
    p.color().is_some_and(|c| c.to_rgb()[0] > 0.3 && c.to_rgb()[1] < 0.1)
}

/// The groups of a face and the parts of the side's graphics drawn on it, each part's outline.
fn faces_with_parts(art: &Node) -> Vec<Vec<PathData>> {
    let mut out = vec![];
    art.walk(&mut |n| {
        if let NodeKind::Group { children, clip: false } = &n.kind
            && let Some((face, parts)) = children.split_first()
            && matches!(face.kind, NodeKind::Path { .. })
            && !parts.is_empty()
        {
            let red: Vec<PathData> = parts
                .iter()
                .filter(|p| p.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if is_red(&f.paint))))
                .filter_map(|p| p.path_data().cloned())
                .collect();
            if red.len() == parts.len() {
                out.push(red);
            }
        }
    });
    out
}

fn bounds(parts: &[Vec<PathData>]) -> Rect {
    parts.iter().flatten().filter_map(PathData::bounds).reduce(|a, b| a.union(b)).unwrap()
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() < 1e-6)
}

/// x runs along the outline from the top's middle, clockwise as the front is seen, and y from the
/// middle of the side toward the back.
#[test]
fn a_side_graphic_lies_on_the_wall_at_its_offset() {
    let d = document();
    let n = object(json!({"graphics": [{"symbol": "Mark", "face": "side"}]}));
    let art = solid_art_in(Some(&d), &n).unwrap();
    let found = faces_with_parts(&art);
    assert_eq!(found.len(), 1, "drawn once, on the top wall");
    // Seen from above, the top wall runs from its front edge (page y 75) to its back (page y 25).
    assert!(near(bounds(&found), Rect::new(45.0, 45.0, 55.0, 55.0)), "{:?}", bounds(&found));
    let moved = object(json!({"graphics": [{"symbol": "Mark", "face": "side", "x": 20.0, "y": 10.0, "scale": 200.0}]}));
    let found = faces_with_parts(&solid_art_in(Some(&d), &moved).unwrap());
    assert!(near(bounds(&found), Rect::new(60.0, 30.0, 80.0, 50.0)), "{:?}", bounds(&found));
    // Without the document nothing is drawn; a graphic on the side leaves the caps without one.
    assert!(faces_with_parts(&solid_art_in(None, &n).unwrap()).is_empty());
}

/// A graphic that reaches past a corner runs on around it onto the next wall.
#[test]
fn a_side_graphic_runs_around_a_corner() {
    let d = document();
    // x = 50 is the top's right end.
    let from_above = object(json!({"graphics": [{"symbol": "Mark", "face": "side", "x": 50.0}]}));
    let found = faces_with_parts(&solid_art_in(Some(&d), &from_above).unwrap());
    assert!(near(bounds(&found), Rect::new(95.0, 45.0, 100.0, 55.0)), "only the top's part shows: {:?}", bounds(&found));
    // Turned to show the top and the right, both walls draw their part.
    let turned = object(json!({"rotationX": -30.0, "rotationY": -40.0, "graphics": [{"symbol": "Mark", "face": "side", "x": 50.0}]}));
    assert_eq!(faces_with_parts(&solid_art_in(Some(&d), &turned).unwrap()).len(), 2);
    // Twist and taper carry the graphic; it stays on the side.
    let twisted =
        object(json!({"rotationX": -30.0, "rotationY": -40.0, "twist": 60.0, "taper": 50.0, "graphics": [{"symbol": "Mark", "face": "side"}]}));
    let found = faces_with_parts(&solid_art_in(Some(&d), &twisted).unwrap());
    assert!(!found.is_empty() && found.iter().flatten().all(|p| p.bounds().is_some_and(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()))));
}

/// The handle lands where the graphic's center is on the visible side, and a page point there
/// gives back its offset; a graphic on a hidden wall has no handle.
#[test]
fn the_sides_handle_moves_the_graphic_on_its_visible_part() {
    let d = document();
    let g = MappedGraphic::from_value(&json!({"symbol": "Mark", "face": "side", "x": 10.0, "y": -5.0})).unwrap();
    let n = object(json!({"graphics": [g.to_value()]}));
    let frame = graphic_frame(Some(&d), &n, None).unwrap();
    let (at, visible) = frame.handle(&g).unwrap();
    assert!(visible && at.distance(Point::new(60.0, 55.0)) < 1e-6, "{at:?}");
    let (x, y) = frame.offset_at(PlanarFace::Side, Point::new(40.0, 45.0)).unwrap();
    assert!((x + 10.0).abs() < 1e-6 && (y - 5.0).abs() < 1e-6, "{x} {y}");
    // The bottom's middle, half way around, is hidden from above; so is a page point off the top.
    let hidden = MappedGraphic { x: 200.0, ..g.clone() };
    assert!(frame.handle(&hidden).is_none());
    assert!(frame.offset_at(PlanarFace::Side, Point::new(50.0, 90.0)).is_none());
    // Without a graphic on the side, the frame places no handle there.
    let caps = graphic_frame(Some(&d), &object(json!({"graphics": [{"symbol": "Mark"}]})), None).unwrap();
    assert!(caps.handle(&g).is_none());
}

/// On a curved wall, whose pieces are cut to its curved outline, the handle sits on the graphic's
/// center and a page point there gives back its offset.
#[test]
fn the_handle_follows_a_curved_side() {
    let d = document();
    let g = MappedGraphic::from_value(&json!({"symbol": "Mark", "face": "side", "x": 6.0, "y": 3.0})).unwrap();
    let mut n = object(json!({"rotationX": -60.0, "graphics": [g.to_value()]}));
    if let NodeKind::Path { path, .. } = &mut n.kind {
        *path = shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0));
    }
    let frame = graphic_frame(Some(&d), &n, None).unwrap();
    let (at, visible) = frame.handle(&g).unwrap();
    assert!(visible);
    let (x, y) = frame.offset_at(PlanarFace::Side, at).unwrap();
    assert!((x - 6.0).abs() < 1e-6 && (y - 3.0).abs() < 1e-6, "{x} {y}");
}

/// A hollow object has no caps but takes graphics on its side; a stroke's solid, whose outline is
/// the longest, takes them once.
#[test]
fn a_hollow_or_stroked_object_takes_a_side_graphic() {
    let d = document();
    let hollow = object(json!({"cap": false, "graphics": [{"symbol": "Mark", "face": "side"}, {"symbol": "Mark"}]}));
    let art = solid_art_in(Some(&d), &hollow).unwrap();
    assert!(near(bounds(&faces_with_parts(&art)), Rect::new(45.0, 45.0, 55.0, 55.0)));
    let mut stroked = object(json!({"graphics": [{"symbol": "Mark", "face": "side"}]}));
    stroked.appearance.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::rgb(0.2, 0.2, 0.2)), 10.0)));
    let found = faces_with_parts(&solid_art_in(Some(&d), &stroked).unwrap());
    assert_eq!(found.len(), 1, "on the stroke's outer wall only");
    assert!(near(bounds(&found), Rect::new(45.0, 45.0, 55.0, 55.0)), "{:?}", bounds(&found));
    // The stroke's outer outline is 440 pt long, cut at a bottom corner 165 pt each way from the
    // top's middle: a new graphic fits the 330 pt about x = 0 and the depth.
    assert_eq!(graphic_side_size(Some(&d), &stroked, None, 0), Some((330.0, 50.0)));
}

/// A stroke's face cut where it passes through a wall keeps the whole face's place on the side, and
/// each piece draws only the part of the graphic within its own outline.
#[test]
fn a_piece_cut_from_a_face_draws_its_own_part() {
    let d = document();
    let facet = |x: f64, normal: [f64; 3], offset: f64| {
        let square = vec![Point::new(x, 0.0), Point::new(x + 10.0, 0.0), Point::new(x + 10.0, 10.0), Point::new(x, 10.0)];
        let uv = square.iter().map(|p| Point::new(p.x - x, p.y)).collect();
        Facet {
            contours: vec![square.clone()],
            depth: 0.0,
            brightness: 1.0,
            illumination: [1.0; 3],
            normal,
            rgb_gradient: None,
            outline: None,
            side: true,
            away: false,
            end: None,
            surface: Some(vectorcraft_three_d::SurfaceCell { page: square, uv, outward: true }),
            membrane: None,
            plane: Some(vectorcraft_three_d::DepthPlane { normal, offset, center: Point::ZERO, eye: None }),
            cell: None,
        }
    };
    let wall = facet(0.0, [0.0, 0.0, 1.0], 0.0);
    // z = x - 8: behind the wall left of x = 8, in front of it right of it.
    let r = std::f64::consts::FRAC_1_SQRT_2;
    let mut stroke = vec![facet(5.0, [-r, 0.0, r], -8.0 * r)];
    super::super::cut_through(std::slice::from_ref(&wall), &mut stroke, &mut 10, &mut 0);
    assert_eq!(stroke.len(), 2, "cut where it passes through the wall");
    // The graphic covers the whole face, x 5 to 15 on the page.
    let graphics = mapped_graphics(&json!({"graphics": [{"symbol": "Mark", "face": "side", "x": 5.0, "y": 5.0}]}));
    let shade = |_: &Facet, c: Color| Paint::solid(c);
    let parts = surface_parts(Some(&d), NodeId(0), (&graphics, PlanarFace::Side), &stroke, &shade, &mut 0).unwrap();
    let drawn: Vec<Rect> =
        parts.iter().map(|on| on.iter().filter_map(|n| n.path_data().and_then(PathData::bounds)).reduce(|a, b| a.union(b)).unwrap()).collect();
    // Each piece draws its own side of x = 8 (where the wall cuts it), together the whole graphic.
    let (left, right) = (drawn[0].x0.min(drawn[1].x0), drawn[0].x1.max(drawn[1].x1));
    assert!((left - 5.0).abs() < 1e-6 && (right - 15.0).abs() < 1e-6, "{drawn:?}");
    assert!(drawn.iter().all(|b| (b.x0 - 8.0).abs() < 1e-6 || (b.x1 - 8.0).abs() < 1e-6), "{drawn:?}");
}

/// Commands refuse a side graphic whose curves, cut into straight pieces on the side, pass the
/// limit of points, though its own path elements are few; on a cap it is accepted.
#[test]
fn a_side_graphic_past_its_limits_is_refused() {
    let mut d = document();
    // 200 circles 200,000 pt across: a few elements each, hundreds of thousands of points cut.
    let mut subpaths = vec![];
    for k in 0..200 {
        let x = f64::from(k);
        subpaths.extend(shapes::ellipse(Rect::new(x, 0.0, x + 2e5, 2e5)).subpaths);
    }
    let rings = Node::path(NodeId(0), PathData::new(subpaths), Appearance::basic(Paint::solid(red()), Paint::None, 0.0));
    d.symbols.push(Symbol { dynamic: false, name: "Rings".into(), art: Arc::new(rings) });
    let n = object(json!({}));
    let side = json!({"rotationX": -90.0, "graphics": [{"symbol": "Rings", "face": "side"}]});
    assert!(validate_three_d_in(Some(&d), &n, EXTRUDE, &side).is_err());
    assert!(validate_three_d_in(Some(&d), &n, EXTRUDE, &json!({"rotationX": -90.0, "graphics": [{"symbol": "Rings"}]})).is_ok());
    assert!(validate_three_d_in(Some(&d), &n, EXTRUDE, &json!({"graphics": [{"symbol": "Mark", "face": "side"}]})).is_ok());
}

/// With the shadow on, the solid that carries the side's graphics is built once, for both its faces
/// and its shadow: the graphic keeps its place on the wall, and the shadow is the one the object
/// casts without a graphic.
#[test]
fn a_side_graphic_keeps_its_place_with_the_shadow_on() {
    let d = document();
    let shadow = |art: &Node| {
        let first = &art.children().unwrap()[0];
        let [AppearanceItem::Fill(fill)] = first.appearance.items.as_slice() else { panic!("no shadow first: {first:?}") };
        assert!(fill.paint.color() == Some(Color::BLACK) && (fill.opacity - 0.5).abs() < 1e-6, "{fill:?}");
        first.path_data().and_then(PathData::bounds).unwrap()
    };
    let lit = json!({"shade": true, "lightHeight": 90.0, "shadow": true, "shadowDistance": 50.0});
    let mut with_graphic = lit.clone();
    with_graphic["graphics"] = json!([{"symbol": "Mark", "face": "side"}]);
    let n = object(with_graphic);
    super::super::SOLID_GEOMETRY_BUILT.with(|b| b.set(0));
    super::super::SOLIDS_BUILT.with(|b| b.set(0));
    let art = solid_art_in(Some(&d), &n).unwrap();
    let builds = (super::super::SOLID_GEOMETRY_BUILT.with(std::cell::Cell::get), super::super::SOLIDS_BUILT.with(std::cell::Cell::get));
    assert_eq!(builds, (1, 1), "one build for the faces and the shadow");
    let found = faces_with_parts(&art);
    assert_eq!(found.len(), 1, "drawn once, on the top wall");
    assert!(near(bounds(&found), Rect::new(45.0, 45.0, 55.0, 55.0)), "{:?}", bounds(&found));
    let without = solid_art_in(Some(&d), &object(lit)).unwrap();
    assert!(near(shadow(&art), shadow(&without)), "{:?} {:?}", shadow(&art), shadow(&without));
}
