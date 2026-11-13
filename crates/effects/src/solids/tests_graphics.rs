use serde_json::json;
use vectorcraft_doc::color::Color;
use vectorcraft_doc::{StrokeLayer, Symbol};
use vectorcraft_geom::shapes;

use super::super::{EXTRUDE, INFLATE, ROTATE, graphic_frame, solid_art_in, three_d_params_as, validate_three_d_in, validate_three_d_params};
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

/// A gray 100 × 100 square at (0, 0) with effect `id`, its parameters `params` over a view
/// straight on.
fn object(id: &str, params: Value) -> Node {
    let mut p = json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": false});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), params.as_object()) {
        p.extend(extra.clone());
    }
    let mut n = square(7, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(Color::rgb(0.5, 0.5, 0.5)));
    n.appearance.effects.push(crate::new_effect(id, &p).unwrap());
    n
}

/// The graphics clip groups in `art`, in drawing order: each one's mapped paths.
fn graphics_in(art: &Node) -> Vec<Vec<(PathData, Paint)>> {
    let mut out = vec![];
    art.walk(&mut |n| {
        if let NodeKind::Group { children, clip: true } = &n.kind
            && let [_, inner] = children.as_slice()
            && let NodeKind::Group { children: paths, clip: false } = &inner.kind
        {
            let paths: Vec<(PathData, Paint)> = paths
                .iter()
                .filter_map(|p| {
                    let fill = p.appearance.items.iter().find_map(|i| match i {
                        AppearanceItem::Fill(f) => Some(f.paint.clone()),
                        AppearanceItem::Stroke(_) => None,
                    })?;
                    Some((p.path_data()?.clone(), fill))
                })
                .collect();
            if paths.iter().any(|(_, p)| p.color().is_some_and(|c| c.to_rgb()[1] < 0.1 && c.to_rgb()[0] > 0.3)) {
                out.push(paths);
            }
        }
    });
    out
}

fn bounds_of(paths: &[(PathData, Paint)]) -> Rect {
    paths.iter().filter_map(|(p, _)| p.bounds()).reduce(|a, b| a.union(b)).unwrap()
}

fn close(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() < 1e-6)
}

#[test]
fn a_graphic_is_drawn_on_the_front_cap_at_its_offset_size_and_turn() {
    let d = document();
    let n = object(EXTRUDE, json!({"graphics": [{"symbol": "Mark", "x": 20.0, "y": -10.0, "scale": 200.0}]}));
    let art = solid_art_in(Some(&d), &n).unwrap();
    let found = graphics_in(&art);
    assert_eq!(found.len(), 1);
    // Straight on, the front cap is the art: a 20 × 20 square centered 20 right of and 10 above (50, 50).
    assert!(close(bounds_of(&found[0]), Rect::new(60.0, 30.0, 80.0, 50.0)), "{:?}", bounds_of(&found[0]));
    // Turned 45°, the square's corners reach out to its half diagonal.
    let n = object(EXTRUDE, json!({"graphics": [{"symbol": "Mark", "rotation": 45.0}]}));
    let b = bounds_of(&graphics_in(&solid_art_in(Some(&d), &n).unwrap())[0]);
    assert!((b.width() - 10.0 * 2f64.sqrt()).abs() < 1e-6);
}

#[test]
fn without_the_document_or_its_symbol_nothing_is_drawn() {
    let n = object(EXTRUDE, json!({"graphics": [{"symbol": "Mark"}]}));
    let art = solid_art_in(None, &n).unwrap();
    assert!(graphics_in(&art).is_empty());
    let n = object(EXTRUDE, json!({"graphics": [{"symbol": "Gone"}]}));
    assert!(graphics_in(&solid_art_in(Some(&document()), &n).unwrap()).is_empty());
    // Without graphics, the art is the same as before they existed.
    let plain = object(EXTRUDE, json!({}));
    assert_eq!(solid_art_in(Some(&document()), &plain), solid_art_in(None, &plain));
}

#[test]
fn the_graphic_is_shaded_with_its_face() {
    let d = document();
    let n = object(EXTRUDE, json!({"shade": true, "lightIntensity": 0.0, "ambient": 50.0, "graphics": [{"symbol": "Mark"}]}));
    let found = graphics_in(&solid_art_in(Some(&d), &n).unwrap());
    let c = found[0][0].1.color().unwrap().to_rgb();
    assert!((c[0] - 0.5).abs() < 1e-3 && c[1].abs() < 1e-6 && c[2].abs() < 1e-6, "{c:?}");
}

#[test]
fn the_graphic_goes_right_after_its_cap_and_is_clipped_to_it() {
    let d = document();
    let n = object(EXTRUDE, json!({"rotationX": -20.0, "rotationY": 25.0, "graphics": [{"symbol": "Mark", "scale": 2000.0}]}));
    let art = solid_art_in(Some(&d), &n).unwrap();
    let NodeKind::Group { children, .. } = &art.kind else { panic!("a group") };
    let NodeKind::Group { children: faces, .. } = &children.last().unwrap().kind else { panic!("the ordered faces") };
    let last = faces.last().unwrap();
    assert!(matches!(&last.kind, NodeKind::Group { clip: true, .. }), "the graphic is drawn last, over the front cap");
    let NodeKind::Group { children: parts, .. } = &last.kind else { panic!() };
    let clip = parts[0].path_data().unwrap().bounds().unwrap();
    let cap = faces[faces.len() - 2].path_data().unwrap().bounds().unwrap();
    assert!(close(clip, cap), "clipped to the cap: {clip:?} against {cap:?}");
}

#[test]
fn the_back_cap_reads_the_right_way_round_from_behind() {
    let d = document();
    let behind = json!({"rotationY": 180.0, "graphics": [{"symbol": "Mark", "face": "back", "x": 20.0}]});
    let n = object(EXTRUDE, behind);
    let found = graphics_in(&solid_art_in(Some(&d), &n).unwrap());
    assert_eq!(found.len(), 1);
    let b = bounds_of(&found[0]);
    // Seen from behind, x runs to the right on the page too.
    assert!((b.center().x - 70.0).abs() < 1e-6, "{b:?}");
    let frame = graphic_frame(Some(&d), &n, None).unwrap();
    let g = MappedGraphic::from_value(&json!({"symbol": "Mark", "face": "back", "x": 20.0})).unwrap();
    let (at, visible) = frame.handle(&g).unwrap();
    assert!(visible && (at.x - 70.0).abs() < 1e-6);
    let (x, y) = frame.offset_at(PlanarFace::Back, Point::new(40.0, 55.0)).unwrap();
    assert!((x + 10.0).abs() < 1e-6 && (y - 5.0).abs() < 1e-6);
    // From the front the back cap is hidden, and so is its graphic.
    let front = object(EXTRUDE, json!({"graphics": [{"symbol": "Mark", "face": "back"}]}));
    assert!(graphics_in(&solid_art_in(Some(&d), &front).unwrap()).is_empty());
}

#[test]
fn a_perspective_foreshortens_the_graphic_with_its_face() {
    let d = document();
    let params = json!({"rotationX": -30.0, "rotationY": 35.0, "perspectiveAngle": 100.0, "graphics": [{"symbol": "Mark", "scale": 400.0}]});
    let n = object(EXTRUDE, params);
    let found = graphics_in(&solid_art_in(Some(&d), &n).unwrap());
    let frame = graphic_frame(Some(&d), &n, None).unwrap();
    let map = frame.map(PlanarFace::Front).unwrap();
    // The corners of the 40 × 40 square centered on the object land where the face's map takes them.
    let path = &found[0][0].0;
    let corners: Vec<Point> = [(30.0, 30.0), (70.0, 30.0), (70.0, 70.0), (30.0, 70.0)].map(|(x, y)| map.to_page(Point::new(x, y)).unwrap()).into();
    for c in corners {
        let near = path.subpaths.iter().flat_map(|s| &s.anchors).map(|a| a.p.distance(c)).fold(f64::INFINITY, f64::min);
        assert!(near < 1e-6, "{c:?} is {near} from the drawn graphic");
    }
}

#[test]
fn a_graphic_behind_the_eye_leaves_the_others_drawn() {
    let d = document();
    let view = json!({"rotationX": -60.0, "rotationY": 0.0, "perspectiveAngle": 160.0});
    let n = object(EXTRUDE, view.clone());
    let frame = graphic_frame(Some(&d), &n, None).unwrap();
    let map = frame.map(PlanarFace::Front).unwrap();
    let c = frame.bounds.center();
    // An offset whose point of the front's plane is at or behind the eye.
    let y = [1.0, -1.0]
        .into_iter()
        .flat_map(|s| (1..60).map(move |k| s * 10f64.powf(f64::from(k) / 10.0)))
        .find(|y| map.local_affine(Point::new(c.x, c.y + y)).is_none())
        .expect("an offset behind the eye");
    let mut params = view;
    params["graphics"] = json!([{"symbol": "Mark"}, {"symbol": "Mark", "y": y}]);
    let found = graphics_in(&solid_art_in(Some(&d), &object(EXTRUDE, params)).unwrap());
    assert_eq!(found.len(), 1, "the graphic that projects is still drawn");
    // A graphic centered there but large enough to reach over the cap draws the part in front.
    let mut params = json!({"rotationX": -60.0, "rotationY": 0.0, "perspectiveAngle": 160.0});
    params["graphics"] = json!([{"symbol": "Mark", "y": y, "scale": (20.0 * (y.abs() + 50.0)).clamp(1.0, 10_000.0)}]);
    let found = graphics_in(&solid_art_in(Some(&d), &object(EXTRUDE, params)).unwrap());
    assert_eq!(found.len(), 1, "the part in front is drawn (offset {y})");
    assert!(!found[0].is_empty());
}

#[test]
fn the_graphic_takes_the_opacity_of_its_fill() {
    let d = document();
    for id in [EXTRUDE, ROTATE] {
        let mut n = object(id, json!({"graphics": [{"symbol": "Mark"}]}));
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
            f.opacity = 0.25;
        }
        let art = solid_art_in(Some(&d), &n).unwrap();
        let mut opacities = vec![];
        art.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Group { clip: true, .. }) {
                opacities.push(c.opacity);
            }
        });
        assert_eq!(opacities, vec![0.25], "{id}");
    }
}

#[test]
fn the_graphic_takes_the_blending_mode_of_its_fill() {
    let d = document();
    for id in [EXTRUDE, ROTATE] {
        let mut n = object(id, json!({"graphics": [{"symbol": "Mark"}]}));
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
            f.blend = BlendMode::Multiply;
        }
        let art = solid_art_in(Some(&d), &n).unwrap();
        let mut blends = vec![];
        art.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Group { clip: true, .. }) {
                blends.push(c.blend);
            }
        });
        assert_eq!(blends, vec![BlendMode::Multiply], "{id}");
    }
}

#[test]
fn each_fill_of_a_plane_gives_its_graphic_its_own_opacity() {
    let d = document();
    let mut n = object(ROTATE, json!({"graphics": [{"symbol": "Mark"}]}));
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.opacity = 0.25;
    }
    let mut second = FillLayer::new(Paint::solid(Color::rgb(0.2, 0.2, 0.2)));
    second.opacity = 0.75;
    n.appearance.items.push(AppearanceItem::Fill(second));
    let art = solid_art_in(Some(&d), &n).unwrap();
    let mut opacities = vec![];
    art.walk(&mut |c| {
        if matches!(c.kind, NodeKind::Group { clip: true, .. }) {
            opacities.push(c.opacity);
        }
    });
    opacities.sort_by(f32::total_cmp);
    assert_eq!(opacities, vec![0.25, 0.75]);
}

#[test]
fn a_symbols_gradient_takes_the_size_of_its_art() {
    use vectorcraft_doc::color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let mut d = document();
    // Symbol art kept in its 20 × 20 square, drawn 40 × 20.
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, red()), GradientStop::new(1.0, Color::BLACK)]));
    g.geom = Some(GradientGeom { start: Point::new(-10.0, 0.0), end: Point::new(10.0, 0.0), aspect: 1.0, focal: None });
    let art = square(0, Rect::new(-10.0, -10.0, 10.0, 10.0), Paint::Gradient(Box::new(g)));
    d.symbols.push(Symbol { dynamic: false, name: "Ramp".into(), art: Arc::new(art) });
    d.unknown.insert(vectorcraft_doc::SYMBOL_SIZES.into(), json!({"Ramp": [40.0, 20.0]}));
    let n = object(EXTRUDE, json!({"graphics": [{"symbol": "Ramp"}]}));
    let out = solid_art_in(Some(&d), &n).unwrap();
    let mut found = None;
    out.walk(&mut |c| {
        if let (Some(p), Some(AppearanceItem::Fill(f))) = (c.path_data(), c.appearance.items.first())
            && let Paint::Gradient(g) = &f.paint
        {
            found = Some((p.bounds().unwrap(), g.geom.unwrap()));
        }
    });
    let (bounds, geom) = found.unwrap();
    assert!(close(bounds, Rect::new(30.0, 40.0, 70.0, 60.0)), "{bounds:?}");
    assert!(geom.start.distance(Point::new(30.0, 50.0)) < 1e-6 && geom.end.distance(Point::new(70.0, 50.0)) < 1e-6, "{geom:?}");
}

#[test]
fn a_symbols_pattern_follows_the_graphic() {
    let mut d = document();
    let tiles = Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY };
    d.symbols.push(Symbol { dynamic: false, name: "Tiles".into(), art: Arc::new(square(0, Rect::new(-10.0, -10.0, 10.0, 10.0), tiles)) });
    let n = object(EXTRUDE, json!({"graphics": [{"symbol": "Tiles", "scale": 200.0}]}));
    let out = solid_art_in(Some(&d), &n).unwrap();
    let mut found = None;
    out.walk(&mut |c| {
        if let Some(AppearanceItem::Fill(f)) = c.appearance.items.first()
            && let Paint::Pattern { xf, .. } = &f.paint
        {
            found = Some((c.path_data().unwrap().bounds().unwrap(), *xf));
        }
    });
    let (bounds, xf) = found.unwrap();
    // The 20 × 20 art drawn 40 × 40 on the middle of the front: its tiles go with it.
    assert!(close(bounds, Rect::new(30.0, 30.0, 70.0, 70.0)), "{bounds:?}");
    assert!((xf * Point::new(-10.0, -10.0)).distance(Point::new(30.0, 30.0)) < 1e-6, "{xf:?}");
    assert!((xf * Point::new(10.0, 10.0)).distance(Point::new(70.0, 70.0)) < 1e-6, "{xf:?}");
}

#[test]
fn a_curve_in_front_of_the_eye_maps_though_its_control_points_do_not() {
    let bounds = Rect::new(10.0, 20.0, 70.0, 60.0);
    let view = vectorcraft_three_d::View { rotation_x: -89.0, rotation_y: 40.0, rotation_z: 10.0, perspective: 160.0, ..Default::default() };
    let map = vectorcraft_three_d::plane_face_map(bounds, view).unwrap();
    // Where the page map stops along the line x = `x`, between the center and a point it does not map.
    let edge = |x: f64| {
        let inside = Point::new(x, bounds.center().y);
        let far = [Point::new(x, -1e6), Point::new(x, 1e6)].into_iter().find(|p| map.to_page(*p).is_none()).unwrap();
        let (mut a, mut b) = (inside, far);
        for _ in 0..200 {
            let m = a.midpoint(b);
            if map.to_page(m).is_some() { a = m } else { b = m }
        }
        a
    };
    let (b1, b2) = (edge(40.0), edge(60.0));
    let u = (b2 - b1).normalize();
    let mut n = vectorcraft_geom::Vec2::new(-u.y, u.x);
    if map.to_page(b1 + n * 10.0).is_none() {
        n = -n;
    }
    // Ends 219.17 in front of that line and control points 100 and 42 past it: the curve stays
    // about 1 in front, while a control point of one of the pieces it is cut into falls behind.
    let at = |s: f64, t: f64| b1 + n * s + u * t;
    let mut curve = BezPath::new();
    curve.move_to(at(219.17, 0.0));
    curve.curve_to(at(-100.0, 5.0), at(-42.0, 15.0), at(219.17, 20.0));
    let c = CubicBez::new(at(219.17, 0.0), at(-100.0, 5.0), at(-42.0, 15.0), at(219.17, 20.0));
    assert!((0..=1000).all(|k| map.to_page(c.eval(k as f64 / 1000.0)).is_some()), "the curve itself is in front");
    let pieces = (0..PERSPECTIVE_PIECES).map(|k| c.subsegment(k as f64 / 8.0..(k + 1) as f64 / 8.0));
    assert!(pieces.flat_map(|s| [s.p1, s.p2]).any(|p| map.to_page(p).is_none()), "a piece's control point is behind");
    let mapped = map_path(&PathData::from_bezpath(&curve), &map, None, usize::MAX).expect("the curve maps");
    // The split pieces count toward the room the path is given: without room, the path is full,
    // not behind the eye, so it is not clipped.
    assert!(mapped.elements().len() > 1 + PERSPECTIVE_PIECES);
    assert_eq!(map_path(&PathData::from_bezpath(&curve), &map, None, PERSPECTIVE_PIECES + 1).err(), Some(NotMapped::Full));
    assert!(mapped.elements().iter().all(|e| e.end_point().is_none_or(|p| p.x.is_finite() && p.y.is_finite())));
    // That piece is split until its halves' control points map, so it stays a curve.
    assert!(mapped.elements().iter().all(|e| !matches!(e, PathEl::LineTo(_))), "{mapped:?}");
    // With its ends at 214.25 instead, the curve dips about 0.24 behind the line between the ends
    // of two pieces, which still map: it has no page point there, and is left to the clipping.
    let c = CubicBez::new(at(214.25, 0.0), at(-100.0, 5.0), at(-42.0, 15.0), at(214.25, 20.0));
    assert!((0..=PERSPECTIVE_PIECES).all(|k| map.to_page(c.eval(k as f64 / 8.0)).is_some()));
    assert!((0..=1000).any(|k| map.to_page(c.eval(k as f64 / 1000.0)).is_none()));
    let mut dips = BezPath::new();
    dips.move_to(c.p0);
    dips.curve_to(c.p1, c.p2, c.p3);
    assert_eq!(map_path(&PathData::from_bezpath(&dips), &map, None, usize::MAX).err(), Some(NotMapped::Behind));
}

#[test]
fn members_with_their_own_blending_get_graphics_of_their_own() {
    let d = document();
    let a = square(1, Rect::new(0.0, 0.0, 50.0, 100.0), Paint::solid(Color::rgb(0.5, 0.5, 0.5)));
    let mut b = square(2, Rect::new(50.0, 0.0, 100.0, 100.0), Paint::solid(Color::rgb(0.5, 0.5, 0.5)));
    b.blend = BlendMode::Multiply;
    let mut group = Node::group(NodeId(3), vec![Arc::new(a), Arc::new(b)]);
    let p = json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": false, "graphics": [{"symbol": "Mark", "scale": 1000.0}]});
    group.appearance.effects.push(crate::new_effect(EXTRUDE, &p).unwrap());
    let art = solid_art_in(Some(&d), &group).unwrap();
    let found = graphics_in(&art);
    // One graphic clipped to each member's cap: the second stays inside the member's Multiply group.
    assert_eq!(found.len(), 2);
    let mut blended = 0;
    art.walk(&mut |c| {
        if c.blend == BlendMode::Multiply {
            blended += graphics_in(c).len();
        }
    });
    assert_eq!(blended, 1);
}

#[test]
fn a_graphic_stays_under_a_solid_painted_after_its_own() {
    let d = document();
    let member = |id: u64, r: Rect, color: Color, opacity: f32| {
        let mut n = square(id, r, Paint::solid(color));
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
            f.opacity = opacity;
        }
        Arc::new(n)
    };
    let gray = Color::rgb(0.5, 0.5, 0.5);
    // A and C share their fill's opacity; B, painted between them, has its own and covers part of A.
    let a = member(1, Rect::new(0.0, 0.0, 60.0, 100.0), gray, 1.0);
    let b = member(2, Rect::new(40.0, 0.0, 100.0, 100.0), Color::rgb(0.0, 0.0, 1.0), 0.5);
    let c = member(4, Rect::new(200.0, 0.0, 260.0, 100.0), gray, 1.0);
    let mut group = Node::group(NodeId(3), vec![a, b, c]);
    let p = json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": false, "graphics": [{"symbol": "Mark", "scale": 3000.0}]});
    group.appearance.effects.push(crate::new_effect(EXTRUDE, &p).unwrap());
    let art = solid_art_in(Some(&d), &group).unwrap();
    // In drawing order: 'G' for a graphic, 'B' for a face of B's solid.
    let mut order = String::new();
    art.walk(&mut |n| {
        if matches!(n.kind, NodeKind::Group { clip: true, .. }) && !graphics_in(n).is_empty() {
            order.push('G');
        } else if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first()
            && f.paint.color().is_some_and(|c| c.to_rgb()[2] > 0.9 && c.to_rgb()[0] < 0.1)
        {
            order.push('B');
        }
    });
    // A's graphic is drawn right after A, under B; C's is drawn after B.
    assert_eq!(graphics_in(&art).len(), 3, "{order}");
    assert!(order.starts_with('G'), "{order}");
}

/// [`document`] with symbol "Dense" too: a red polygon of 30,000 sides, straight or `curved`.
fn dense_document(curved: bool) -> Document {
    let mut d = document();
    let mut poly = BezPath::new();
    let mut last = Point::ZERO;
    for k in 0..30_000 {
        let a = k as f64 / 30_000.0 * std::f64::consts::TAU;
        let p = Point::new(5.0 + 5.0 * a.cos(), 5.0 + 5.0 * a.sin());
        if k == 0 {
            poly.move_to(p);
        } else if curved {
            poly.curve_to(last.lerp(p, 1.0 / 3.0), last.lerp(p, 2.0 / 3.0), p);
        } else {
            poly.line_to(p);
        }
        last = p;
    }
    poly.close_path();
    let art = Node::path(NodeId(0), PathData::from_bezpath(&poly), Appearance::basic(Paint::solid(red()), Paint::None, 0.0));
    d.symbols.push(Symbol { dynamic: false, name: "Dense".into(), art: Arc::new(art) });
    d
}

/// The graphics drawn on a group of two members, each with its own graphic of symbol "Dense"
/// ([`dense_document`]), seen under a perspective.
fn dense_graphics_drawn(curved: bool) -> usize {
    let d = dense_document(curved);
    let a = square(1, Rect::new(0.0, 0.0, 50.0, 100.0), Paint::solid(Color::rgb(0.5, 0.5, 0.5)));
    let mut b = square(2, Rect::new(50.0, 0.0, 100.0, 100.0), Paint::solid(Color::rgb(0.5, 0.5, 0.5)));
    b.blend = BlendMode::Multiply;
    let mut group = Node::group(NodeId(3), vec![Arc::new(a), Arc::new(b)]);
    let p = json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "perspectiveAngle": 60.0, "shade": false, "graphics": [{"symbol": "Dense", "scale": 1000.0}]});
    group.appearance.effects.push(crate::new_effect(EXTRUDE, &p).unwrap());
    graphics_in(&solid_art_in(Some(&d), &group).unwrap()).len()
}

#[test]
fn members_with_graphics_of_their_own_share_one_element_limit() {
    // A perspective cuts each of the 30,000 curves into 8: one copy fits the limit, two do not.
    // The first member's graphic is drawn; the second would pass the limit, so it draws none.
    assert_eq!(dense_graphics_drawn(true), 1);
}

#[test]
fn straight_sides_count_once_under_a_perspective() {
    // A perspective keeps each of the 30,000 straight sides one element: both copies fit the limit.
    assert_eq!(dense_graphics_drawn(false), 2);
}

#[test]
fn the_symbol_check_counts_what_the_view_draws() {
    // Two graphics of the 30,000-sided polygon under a perspective: straight, they fit the limit;
    // curved, they do not.
    let two = json!({"perspectiveAngle": 60.0, "graphics": [{"symbol": "Dense"}, {"symbol": "Dense", "face": "back"}]});
    let n = object(EXTRUDE, two.clone());
    assert!(validate_three_d_in(Some(&dense_document(false)), &n, EXTRUDE, &two).is_ok());
    let e = validate_three_d_in(Some(&dense_document(true)), &n, EXTRUDE, &two).unwrap_err();
    assert!(e.contains("too complex"), "{e}");
    // Without a perspective each curve draws once: curved, they fit too.
    let flat = json!({"perspectiveAngle": 0.0, "graphics": two["graphics"].clone()});
    assert!(validate_three_d_in(Some(&dense_document(true)), &n, EXTRUDE, &flat).is_ok());
}

#[test]
fn a_plane_and_a_hollow_solid() {
    let d = document();
    let plane = object(ROTATE, json!({"rotationY": 30.0, "graphics": [{"symbol": "Mark"}]}));
    assert_eq!(graphics_in(&solid_art_in(Some(&d), &plane).unwrap()).len(), 1);
    let hollow = object(EXTRUDE, json!({"cap": false, "graphics": [{"symbol": "Mark"}]}));
    assert!(graphics_in(&solid_art_in(Some(&d), &hollow).unwrap()).is_empty());
}

#[test]
fn a_stroked_graphic_draws_its_stroke_as_an_outline() {
    let mut d = document();
    let mut ring = square(0, Rect::new(0.0, 0.0, 10.0, 10.0), Paint::None);
    ring.appearance = Appearance { items: vec![AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(red()), 2.0))], ..Default::default() }.into();
    d.symbols.push(Symbol { dynamic: false, name: "Ring".into(), art: Arc::new(ring) });
    let n = object(EXTRUDE, json!({"graphics": [{"symbol": "Ring"}]}));
    let found = graphics_in(&solid_art_in(Some(&d), &n).unwrap());
    // The 10 × 10 square's 2 pt stroke reaches 1 pt outside it.
    assert!(close(bounds_of(&found[0]), Rect::new(44.0, 44.0, 56.0, 56.0)), "{:?}", bounds_of(&found[0]));
}

#[test]
fn graphics_are_checked() {
    let n = object(EXTRUDE, json!({}));
    let check = |id: &str, graphics: Value| validate_three_d_in(None, &n, id, &json!({"graphics": graphics}));
    assert!(check(EXTRUDE, json!([{"symbol": "Mark", "face": "back", "x": 3.0, "scale": 50.0, "rotation": -90.0}])).is_ok());
    assert!(check(EXTRUDE, json!([])).is_ok());
    for bad in [
        json!("Mark"),
        json!([{"face": "front"}]),
        json!([{"symbol": ""}]),
        json!([{"symbol": "Mark", "face": "side"}]),
        json!([{"symbol": "Mark", "scale": 0.0}]),
        json!([{"symbol": "Mark", "rotation": 400.0}]),
        json!([{"symbol": "Mark", "x": "1"}]),
        json!([{"symbol": "Mark", "repeat": true}]),
        json!(vec![json!({"symbol": "Mark"}); MAX_GRAPHICS + 1]),
    ] {
        assert!(check(EXTRUDE, bad.clone()).is_err(), "{bad}");
    }
    assert!(check(ROTATE, json!([{"symbol": "Mark", "face": "back"}])).is_err());
    assert!(check(INFLATE, json!([{"symbol": "Mark"}])).is_err());
    // Revolve takes graphics on its side, named the front; Revolve (Classic) takes none.
    assert!(validate_three_d_params(crate::REVOLVE, &json!({"graphics": [{"symbol": "Mark"}]})).is_ok());
    assert!(validate_three_d_params(crate::REVOLVE, &json!({"graphics": [{"symbol": "Mark", "face": "back"}]})).is_err());
    assert!(validate_three_d_params(crate::CLASSIC_REVOLVE, &json!({"graphics": [{"symbol": "Mark"}]})).is_err());
    // A loaded file's invalid entries are left out when drawn.
    assert_eq!(mapped_graphics(&json!({"graphics": [{"symbol": "Mark"}, {"face": "back"}, 3]})).len(), 1);
}

#[test]
fn the_frame_at_an_index_is_that_effects() {
    let d = document();
    let mut n = object(EXTRUDE, json!({}));
    n.appearance.effects.push(crate::new_effect(ROTATE, &json!({})).unwrap());
    // The first 3D effect is the Extrude & Bevel, with a back cap; the second is a plane, without.
    assert!(graphic_frame(Some(&d), &n, None).unwrap().map(PlanarFace::Back).is_some());
    assert!(super::super::graphic_frame_at(Some(&d), &n, None, 0).unwrap().map(PlanarFace::Back).is_some());
    assert!(super::super::graphic_frame_at(Some(&d), &n, None, 1).unwrap().map(PlanarFace::Back).is_none());
    assert!(super::super::graphic_frame_at(Some(&d), &n, None, 2).is_none());
}

#[test]
fn a_change_to_a_plane_drops_the_back_caps_graphics() {
    let p = json!({"graphics": [{"symbol": "Mark", "face": "front"}, {"symbol": "Mark", "face": "back"}]});
    let plane = three_d_params_as(EXTRUDE, &p, ROTATE);
    assert_eq!(plane["graphics"], json!([{"symbol": "Mark", "face": "front"}]));
    assert!(validate_three_d_params(ROTATE, &plane).is_ok());
    // Back to Extrude & Bevel, the front's graphic stays.
    assert_eq!(three_d_params_as(ROTATE, &plane, EXTRUDE)["graphics"], plane["graphics"]);
}

#[test]
fn a_stroked_solid_carries_its_graphic_too() {
    let d = document();
    let mut n = Node::path(
        NodeId(7),
        shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
        Appearance::basic(Paint::None, Paint::solid(Color::rgb(0.5, 0.5, 0.5)), 20.0),
    );
    let p = json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": false, "graphics": [{"symbol": "Mark", "scale": 2000.0}]});
    n.appearance.effects.push(crate::new_effect(EXTRUDE, &p).unwrap());
    assert_eq!(graphics_in(&solid_art_in(Some(&d), &n).unwrap()).len(), 1);
}

#[test]
fn art_nested_too_deep_is_refused_and_not_drawn() {
    let mut d = document();
    let mut art = square(0, Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(red()));
    for k in 0..=MAX_LEVELS {
        art = Node::group(NodeId(k as u64 + 1), vec![Arc::new(art)]);
    }
    d.symbols.push(Symbol { dynamic: false, name: "Deep".into(), art: Arc::new(art) });
    let n = object(EXTRUDE, json!({"graphics": [{"symbol": "Deep"}]}));
    let e = validate_three_d_in(Some(&d), &n, EXTRUDE, &json!({"graphics": [{"symbol": "Deep"}]})).unwrap_err();
    assert!(e.contains("nested"), "{e}");
    assert!(graphics_in(&solid_art_in(Some(&d), &n).unwrap()).is_empty());
    // A loaded file draws its other graphics.
    let both = object(EXTRUDE, json!({"graphics": [{"symbol": "Deep"}, {"symbol": "Mark"}]}));
    assert_eq!(graphics_in(&solid_art_in(Some(&d), &both).unwrap()).len(), 1);
}

#[test]
fn symbols_nested_too_deep_are_refused_and_not_drawn() {
    let mut d = document();
    // Symbol `N{k}` holds an instance of `N{k-1}`, and `N0` is a square.
    d.symbols.push(Symbol { dynamic: false, name: "N0".into(), art: Arc::new(square(0, Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(red()))) });
    for k in 1..=MAX_NESTING + 1 {
        let instance = Node::new(NodeId(k as u64), NodeKind::SymbolInstance { symbol: format!("N{}", k - 1), xf: Affine::IDENTITY });
        d.symbols.push(Symbol { dynamic: false, name: format!("N{k}"), art: Arc::new(instance) });
    }
    let deepest = format!("N{MAX_NESTING}");
    let p = json!({"graphics": [{"symbol": deepest}]});
    assert!(validate_three_d_in(Some(&d), &object(EXTRUDE, p.clone()), EXTRUDE, &p).is_ok());
    assert_eq!(graphics_in(&solid_art_in(Some(&d), &object(EXTRUDE, p)).unwrap()).len(), 1);
    let past = format!("N{}", MAX_NESTING + 1);
    let p = json!({"graphics": [{"symbol": past}]});
    let e = validate_three_d_in(Some(&d), &object(EXTRUDE, p.clone()), EXTRUDE, &p).unwrap_err();
    assert!(e.contains("nest"), "{e}");
    assert!(graphics_in(&solid_art_in(Some(&d), &object(EXTRUDE, p)).unwrap()).is_empty());
}

#[test]
fn a_solid_in_a_transparent_gradient_keeps_its_graphic() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let d = document();
    let mut stop = GradientStop::new(1.0, red());
    stop.opacity = 0.25;
    let gradient = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, red()), stop]));
    // Under a colored light the faces are drawn opaque and masked by the gradient's transparency.
    let mut n = object(EXTRUDE, json!({"shade": true, "lights": [{"color": "#ff8080"}], "graphics": [{"symbol": "Mark"}]}));
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.paint = Paint::Gradient(Box::new(gradient));
    }
    let art = solid_art_in(Some(&d), &n).unwrap();
    let mut masked = false;
    art.walk(&mut |c| masked |= c.extras().mask.is_some());
    assert!(masked);
    assert_eq!(graphics_in(&art).len(), 1);
    // Each graphic clip group in `art`, in drawing order: whether it carries an opacity mask.
    let masks = |art: &Node| {
        let mut out = vec![];
        art.walk(&mut |c| {
            if let NodeKind::Group { children, clip: true } = &c.kind
                && let [_, inner] = children.as_slice()
                && matches!(inner.kind, NodeKind::Group { clip: false, .. })
            {
                out.push(c.extras().mask.is_some());
            }
        });
        out
    };
    // The graphic carries the cap's mask, so it is hidden where the gradient is transparent.
    assert_eq!(masks(&art), vec![true]);
    // With an opaque stroke drawn after the fill, the fill's cap keeps a masked graphic of its own
    // and the stroke's outline gets an unmasked one.
    n.appearance.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 4.0)));
    assert_eq!(masks(&solid_art_in(Some(&d), &n).unwrap()), vec![true, false]);
}

/// The paths of `art`'s faces in drawing order (a face lit by a colored light is a clip group: its
/// clipping path), leaving out the graphics (clip groups of solid-color paths).
fn face_paths(art: &Node) -> Vec<PathData> {
    fn walk(n: &Node, out: &mut Vec<PathData>) {
        match &n.kind {
            NodeKind::Group { children, clip: true } => {
                let graphic = children.get(1).is_some_and(|inner| match &inner.kind {
                    NodeKind::Group { children: paths, clip: false } => paths.iter().all(|p| {
                        p.path_data().is_some()
                            && p.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if f.paint.color().is_some()))
                    }),
                    _ => false,
                });
                if !graphic {
                    out.extend(children.first().and_then(|c| c.path_data()).cloned());
                }
            }
            NodeKind::Group { children, .. } => children.iter().for_each(|c| walk(c, out)),
            _ => out.extend(n.path_data().cloned()),
        }
    }
    let mut out = vec![];
    walk(art, &mut out);
    out
}

/// A turned solid in a gradient with transparent stops under a colored light, with a dashed stroke
/// and a graphic, draws its fill's and its stroke's faces in the one order it draws them in without
/// the graphic, and the graphic keeps the cap's mask and goes after the fill's cap, not the stroke's.
#[test]
fn a_transparent_gradient_with_a_graphic_keeps_the_shared_face_order() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let d = document();
    let mut stop = GradientStop::new(1.0, red());
    stop.opacity = 0.25;
    let gradient = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, red()), stop]));
    let solid = |graphics: Value| {
        let mut n = object(
            EXTRUDE,
            json!({"rotationX": -20.0, "rotationY": -40.0, "depth": 60.0, "shade": true, "lights": [{"color": "#ff8080"}], "graphics": graphics}),
        );
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
            f.paint = Paint::Gradient(Box::new(gradient.clone()));
        }
        // A gradient stroke: a solid color under the colored light would be one composite.
        let blues = Gradient::new(
            GradientKind::Linear,
            vec![GradientStop::new(0.0, Color::rgb(0.0, 0.0, 1.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 0.5))],
        );
        let mut stroke = StrokeLayer::new(Paint::Gradient(Box::new(GradientPaint::new(blues))), 6.0);
        stroke.dash = Some(vectorcraft_doc::Dash { pattern: vec![12.0, 12.0], offset: 0.0, align_corners: false });
        n.appearance.items.push(AppearanceItem::Stroke(stroke));
        solid_art_in(Some(&d), &n).unwrap()
    };
    let with = solid(json!([{"symbol": "Mark"}]));
    let without = solid(json!([]));
    assert!(face_paths(&without).len() > 20, "the faces are found");
    assert_eq!(face_paths(&with), face_paths(&without), "the faces in one order");
    // The graphic is drawn on the fill's cap and on the stroke's (the clip groups holding the
    // symbol's red square); the fill's, whose surface carries the mask, carries it too.
    let mut masked = vec![];
    with.walk(&mut |c| {
        if let NodeKind::Group { children, clip: true } = &c.kind
            && let [_, inner] = children.as_slice()
            && let NodeKind::Group { children: paths, clip: false } = &inner.kind
            && paths.iter().any(|p| {
                p.appearance.items.iter().any(|i| match i {
                    AppearanceItem::Fill(f) => f.paint.color().is_some_and(|c| c.to_rgb()[1] < 0.1 && c.to_rgb()[0] > 0.3),
                    AppearanceItem::Stroke(_) => false,
                })
            })
        {
            masked.push(c.extras().mask.is_some());
        }
    });
    masked.sort_unstable();
    assert_eq!(masked, [false, true], "the fill's graphic carries the cap's mask");
    // The fill's graphic goes right after the fill's cap, so the dashes on the cap's edge, drawn
    // after it, cover it. Drawn in order: the fill's graphic (the masked clip group), and each face,
    // the fill's or the stroke's by its gradient's first stop (red or blue), with its bounds.
    let mut drawn: Vec<(&str, Rect)> = vec![];
    with.walk(&mut |c| {
        if let NodeKind::Group { children, clip: true } = &c.kind
            && c.extras().mask.is_some()
            && let Some(bounds) = children.first().and_then(|clip| clip.path_data()).and_then(PathData::bounds)
        {
            drawn.push(("graphic", bounds));
        }
        let first_stop = c.appearance.items.iter().find_map(|i| match i {
            AppearanceItem::Fill(f) => match &f.paint {
                Paint::Gradient(g) => g.gradient.stops.first().map(|s| s.color.to_rgb()),
                _ => None,
            },
            AppearanceItem::Stroke(_) => None,
        });
        if let (Some(rgb), Some(bounds)) = (first_stop, c.path_data().and_then(PathData::bounds)) {
            drawn.push((if rgb[0] > rgb[2] { "fill" } else { "stroke" }, bounds));
        }
    });
    let graphic = drawn.iter().position(|(kind, _)| *kind == "graphic").unwrap();
    let covering = |(kind, b): &(&str, Rect)| *kind == "stroke" && b.intersect(drawn[graphic].1).area() > 0.0;
    assert!(drawn[graphic..].iter().any(covering), "a dash on the cap's edge is drawn over the fill's graphic");
}

#[test]
fn a_cap_with_a_hole_keeps_the_hole_out_of_the_graphic() {
    let d = document();
    let outer = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let hole = shapes::rectangle(Rect::new(30.0, 30.0, 70.0, 70.0)).to_bezpath().reverse_subpaths();
    let mut ring = outer;
    ring.extend(hole.iter());
    for flip in [false, true] {
        let ring = if flip { ring.reverse_subpaths() } else { ring.clone() };
        let mut n =
            Node::path(NodeId(7), PathData::from_bezpath(&ring), Appearance::basic(Paint::solid(Color::rgb(0.5, 0.5, 0.5)), Paint::None, 0.0));
        let p = json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": false, "graphics": [{"symbol": "Mark", "scale": 2000.0}]});
        n.appearance.effects.push(crate::new_effect(EXTRUDE, &p).unwrap());
        let art = solid_art_in(Some(&d), &n).unwrap();
        let mut clips = vec![];
        art.walk(&mut |c| {
            if let NodeKind::Group { children, clip: true } = &c.kind {
                clips.push(children[0].path_data().unwrap().to_bezpath());
            }
        });
        assert_eq!(clips.len(), 1, "flip {flip}");
        assert_ne!(clips[0].winding(Point::new(10.0, 10.0)), 0, "flip {flip}: the ring is inside the clip");
        assert_eq!(clips[0].winding(Point::new(50.0, 50.0)), 0, "flip {flip}: the hole is outside it");
    }
}

#[test]
fn a_graphic_starts_inside_its_object() {
    let d = document();
    let frame = graphic_frame(Some(&d), &object(EXTRUDE, json!({})), None).unwrap();
    assert_eq!(frame.fitting_scale(Rect::new(0.0, 0.0, 10.0, 10.0)), 100.0);
    assert!((frame.fitting_scale(Rect::new(0.0, 0.0, 400.0, 200.0)) - 25.0).abs() < 1e-9);
    assert_eq!(super::super::graphic_bounds(&d, "Mark"), Some(Rect::new(500.0, 500.0, 510.0, 510.0)));
}

#[test]
fn a_symbol_instance_in_a_graphic_draws_as_the_canvas_shows_it() {
    let mut d = document();
    let blue = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    // A dynamic symbol whose instance in "Changed" fills its square blue instead of red.
    d.symbols.push(Symbol { dynamic: true, name: "Dynamic".into(), art: Arc::new(square(0, Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(red()))) });
    let mut changed = Node::new(NodeId(1), NodeKind::SymbolInstance { symbol: "Dynamic".into(), xf: Affine::IDENTITY });
    changed.extras_mut().symbol_overrides =
        vec![vectorcraft_doc::symbol_overrides::SymbolOverride { fill: Some(blue.clone()), ..Default::default() }];
    d.symbols.push(Symbol { dynamic: false, name: "Changed".into(), art: Arc::new(changed) });
    let pieces = pieces_of(&d, d.symbols.iter().find(|s| s.name == "Changed").unwrap()).unwrap();
    assert_eq!(pieces.iter().map(|p| p.paint.clone()).collect::<Vec<_>>(), [blue]);
    // A symbol with 9-slice guides, a 100 × 100 square with a 10 × 10 one in its corner, placed
    // three times as wide: the corner keeps its width while the rest stretches.
    let corner = square(2, Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(red()));
    let whole = square(3, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(red()));
    d.symbols.push(Symbol { dynamic: false, name: "Sliced".into(), art: Arc::new(Node::group(NodeId(4), vec![Arc::new(whole), Arc::new(corner)])) });
    d.set_symbol_nine_slice("Sliced", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let wide = Node::new(NodeId(5), NodeKind::SymbolInstance { symbol: "Sliced".into(), xf: Affine::scale_non_uniform(3.0, 1.0) });
    d.symbols.push(Symbol { dynamic: false, name: "Wide".into(), art: Arc::new(wide) });
    let pieces = pieces_of(&d, d.symbols.iter().find(|s| s.name == "Wide").unwrap()).unwrap();
    let widths: Vec<f64> = pieces.iter().map(|p| p.path.bounds().unwrap().width()).collect();
    assert_eq!(widths.len(), 2);
    assert!((widths[1] / widths[0] - 1.0 / 30.0).abs() < 1e-6, "{widths:?}");
    // With a gradient, the square crossing the guides is cut at them: its pieces cover the
    // placed 300 × 100 once, not one whole copy for each cell.
    use vectorcraft_doc::color::{Gradient, GradientPaint};
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let shaded = square(6, Rect::new(0.0, 0.0, 100.0, 100.0), gradient);
    d.symbols.push(Symbol { dynamic: false, name: "Shaded".into(), art: Arc::new(Node::group(NodeId(7), vec![Arc::new(shaded)])) });
    d.set_symbol_nine_slice("Shaded", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let wide = Node::new(NodeId(8), NodeKind::SymbolInstance { symbol: "Shaded".into(), xf: Affine::scale_non_uniform(3.0, 1.0) });
    d.symbols.push(Symbol { dynamic: false, name: "WideShaded".into(), art: Arc::new(wide) });
    let pieces = pieces_of(&d, d.symbols.iter().find(|s| s.name == "WideShaded").unwrap()).unwrap();
    let area: f64 = pieces.iter().map(|p| p.path.to_bezpath().area().abs()).sum();
    assert!((area - 30_000.0).abs() < 1e-3, "{area}");
}

#[test]
fn curves_are_cut_one_at_a_time_and_the_work_stops_at_the_limit() {
    let ellipse = shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 6.0)).to_bezpath();
    // Cut curve by curve, a path gives the points it gives cut whole.
    for scale in [1.0, 100.0, 10_000.0] {
        let placed = Affine::scale(scale) * ellipse.clone();
        let mut whole = vec![];
        vectorcraft_geom::kurbo::flatten(placed.iter(), SURFACE_TOLERANCE, |el| {
            if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
                whole.push(p);
            }
        });
        let mut points = 0;
        let cut: Vec<Point> = straight_pieces(&placed, &mut points).unwrap().into_iter().flatten().collect();
        assert_eq!(points, cut.len());
        assert_eq!(cut.len(), whole.len(), "{scale}");
        assert!(cut.iter().zip(&whole).all(|(a, b)| a.distance(*b) < 1e-9), "{scale}");
    }
    // Past the limit no further curve is cut: of 1,000 ellipses that each take more than 3,000
    // points, the count stops within one curve of the limit.
    let mut many = BezPath::new();
    (0..1000).for_each(|_| many.extend(Affine::scale(10_000.0) * ellipse.clone()));
    let mut points = 0;
    assert!(straight_pieces(&many, &mut points).is_none());
    assert!(points > MAX_ELEMENTS && points < MAX_ELEMENTS + 2000, "{points}");
    // A curve that reaches farther than any surface, or to no finite point, is not cut at all.
    for scale in [1e12, f64::INFINITY] {
        let mut points = 0;
        assert!(straight_pieces(&(Affine::scale(scale) * ellipse.clone()), &mut points).is_none());
        assert!(points <= 1, "{scale}: {points}");
    }
}

#[test]
fn a_line_that_reaches_farther_than_any_surface_is_not_cut() {
    let triangle = |far: f64| {
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.line_to((far, 0.0));
        b.line_to((0.0, 1.0));
        b.close_path();
        b
    };
    assert!(straight_pieces(&triangle(1e7), &mut 0).is_some());
    for far in [1e12, 1e300] {
        assert!(straight_pieces(&triangle(far), &mut 0).is_none(), "{far}");
        assert!(straight_pieces(&(Affine::translate((far, 0.0)) * triangle(1.0)), &mut 0).is_none(), "{far}");
    }
}

#[test]
fn only_an_opaque_part_on_revolve_takes_a_seam_stroke() {
    let piece = |paint: Paint, opacity: f32, blend: BlendMode| Unrolled {
        polygons: vec![],
        boxes: vec![],
        bounds: Rect::ZERO,
        rule: FillRule::NonZero,
        paint,
        opacity,
        blend,
        place: Affine::IDENTITY,
    };
    assert!(takes_seam(&piece(Paint::solid(red()), 1.0, BlendMode::Normal)));
    assert!(!takes_seam(&piece(Paint::solid(red()), 0.5, BlendMode::Normal)));
    assert!(!takes_seam(&piece(Paint::solid(red()), 1.0, BlendMode::Multiply)));
    // A pattern's tile may hold transparent art, which the overlap would show.
    assert!(!takes_seam(&piece(Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY }, 1.0, BlendMode::Normal)));
}
