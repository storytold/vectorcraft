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
    assert!(validate_three_d_params(crate::REVOLVE, &json!({"graphics": [{"symbol": "Mark"}]})).is_err());
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
