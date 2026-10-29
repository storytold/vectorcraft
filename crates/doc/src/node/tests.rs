use super::*;

/// A node writes its rarely set parts among the others, in the file's field order, and reads
/// them back into its extras; a node without any keeps none.
#[test]
fn extras_are_written_flat_in_order() {
    let mut n = Node::path(NodeId(7), shapes::rectangle(Rect::new(0.0, 0.0, 4.0, 2.0)), Appearance::default_art());
    n.name = Some("n".into());
    n.knockout_shape = true;
    n.bbox_angle = 30.0;
    {
        let mut e = n.extras_mut();
        e.mask = Some(Box::new(OpacityMask::new(Node::group(NodeId(8), vec![]), true)));
        e.trace = Some(Box::new(serde_json::json!({"preset": "p"})));
        e.series_index = Some(2);
        e.type_outlines = true;
        e.graphic_style = Some(3);
        e.attrs = Some(Box::default());
        e.slice = Some(Box::default());
        e.perspective = Some(Box::new(crate::PerspectiveAttachment::new("left", 1.0)));
        e.live_paint_gaps = Some(Box::default());
        e.compound_shape = true;
        e.shape_mode = crate::ShapeMode::Exclude;
    }
    /// The top-level keys of a JSON object, in the order written.
    struct Keys(Vec<String>);
    impl<'de> Deserialize<'de> for Keys {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> serde::de::Visitor<'de> for V {
                type Value = Keys;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("an object")
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Keys, A::Error> {
                    let mut out = vec![];
                    while let Some(k) = a.next_key::<String>()? {
                        a.next_value::<serde::de::IgnoredAny>()?;
                        out.push(k);
                    }
                    Ok(Keys(out))
                }
            }
            d.deserialize_map(V)
        }
    }
    let text = serde_json::to_string(&n).unwrap();
    let keys = serde_json::from_str::<Keys>(&text).unwrap().0;
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let order = [
        "id",
        "name",
        "knockout_shape",
        "appearance",
        "mask",
        "trace",
        "series_index",
        "type_outlines",
        "kind",
        "graphic_style",
        "attrs",
        "slice",
        "bbox_angle",
        "perspective",
        "live_paint_gaps",
        "compound_shape",
        "shape_mode",
    ];
    assert_eq!(keys, order);
    assert_eq!(v["shape_mode"], "exclude");
    assert_eq!(serde_json::from_value::<Node>(v).unwrap(), n);
    let plain = Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 4.0, 2.0)), Appearance::default_art());
    let back: Node = serde_json::from_value(serde_json::to_value(&plain).unwrap()).unwrap();
    assert!(back.extras_box().is_none());
    // Clearing the last extra lets them go; clearing one a node doesn't have allocates none.
    n.extras_mut().mask = None;
    assert!(n.extras_box().is_some());
    *n.extras_mut() = NodeExtras::default();
    assert!(n.extras_box().is_none());
    n.extras_mut().mask = None;
    assert!(n.extras_box().is_none());
    assert_eq!(n.extras(), &NodeExtras::default());
}

#[test]
fn rect_live_shape_roundtrip() {
    let l = LiveShape::Rectangle {
        w: 10.0,
        h: 20.0,
        radii: [0.0; 4],
        kinds: Default::default(),
        radii_relative: false,
        xf: Affine::translate((5.0, 5.0)),
    };
    assert_eq!(l.to_path().bounds(), Some(Rect::new(5.0, 5.0, 15.0, 25.0)));
    assert_eq!(l.label(), "Rectangle");
}

#[test]
fn corner_kinds_save_only_when_some_corner_isnt_round() {
    let mut l = LiveShape::Rectangle {
        w: 100.0,
        h: 50.0,
        radii: [0.0, 10.0, 0.0, 0.0],
        kinds: Default::default(),
        radii_relative: false,
        xf: Affine::IDENTITY,
    };
    // Older files read as round.
    let json = serde_json::to_value(&l).unwrap();
    assert!(json.get("kinds").is_none(), "{json}");
    assert_eq!(serde_json::from_value::<LiveShape>(json).unwrap(), l);
    if let LiveShape::Rectangle { kinds, .. } = &mut l {
        kinds[1] = CornerKind::Chamfer;
    }
    let json = serde_json::to_value(&l).unwrap();
    assert_eq!(json["kinds"], serde_json::json!(["round", "chamfer", "round", "round"]));
    assert_eq!(serde_json::from_value::<LiveShape>(json).unwrap(), l);
    // Polygons saved before Live Corners cut theirs read as sharp; a live path round-trips.
    let p: LiveShape = serde_json::from_value(serde_json::json!({"shape": "polygon", "radius": 10.0, "sides": 5, "xf": [1, 0, 0, 1, 0, 0]})).unwrap();
    assert_eq!(p, LiveShape::Polygon { radius: 10.0, sides: 5, xf: Affine::IDENTITY, radii: vec![], kinds: vec![], radii_relative: false });
    let star = shapes::star(Point::new(0.0, 0.0), 20.0, 10.0, 5, 0.0);
    let path = LiveShape::Path { base: star, radii: vec![2.0; 10], kinds: vec![], radii_relative: false };
    let json = serde_json::to_value(&path).unwrap();
    assert_eq!((json["shape"].as_str(), json.get("kinds")), (Some("path"), None));
    assert_eq!(serde_json::from_value::<LiveShape>(json).unwrap(), path);
    assert_eq!(path.label(), "Path");
}

/// A polygon read from a file has 3 to 1000 sides, as the commands give it: selecting one
/// with four billion made its outline and corner lists allocate tens of gigabytes.
#[test]
fn a_polygon_from_a_file_has_3_to_1000_sides() {
    let read = |sides: u32| {
        let json = serde_json::json!({"shape": "polygon", "radius": 10.0, "sides": sides, "xf": [1, 0, 0, 1, 0, 0]});
        serde_json::from_value::<LiveShape>(json).unwrap()
    };
    let sides_of = |l: &LiveShape| match l {
        LiveShape::Polygon { sides, .. } => *sides,
        _ => panic!("not a polygon: {l:?}"),
    };
    let huge = read(4_000_000_000);
    assert_eq!([0, 5, 1000].map(|n| sides_of(&read(n))), [3, 5, 1000]);
    assert_eq!(sides_of(&huge), 1000);
    assert_eq!(huge.polygon_outline().anchor_count(), 1000);
}

#[test]
fn a_polygon_reports_and_takes_its_radius_angle_and_equal_sides() {
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    // A hexagon of radius 10 at (50, 50), drawn 2× and turned 30° clockwise on the page.
    let xf = Affine::translate((50.0, 50.0)) * Affine::rotate(30f64.to_radians()) * Affine::scale(2.0);
    let mut p = LiveShape::Polygon { radius: 10.0, sides: 6, xf, radii: vec![], kinds: vec![], radii_relative: false };
    assert!(close(p.polygon_radius().unwrap(), 20.0) && close(p.polygon_angle().unwrap(), 330.0) && p.polygon_sides_equal());
    p.set_polygon_angle(45.0);
    assert!(close(p.polygon_angle().unwrap(), 45.0));
    let centre = p.to_path().bounds().unwrap().center();
    assert!(close(centre.x, 50.0) && close(centre.y, 50.0), "turned about its centre: {centre:?}");
    p.set_polygon_radius(30.0);
    assert!(close(p.polygon_radius().unwrap(), 30.0));
    // A hexagon's side is as long as its radius.
    assert!(close(p.polygon_radius_for_side(12.0).unwrap(), 12.0));
    // Stretched, its sides differ; Make Sides Equal keeps its centre, angle and mean radius.
    p.transform(Affine::scale_non_uniform(2.0, 1.0));
    assert!(!p.polygon_sides_equal());
    let (r, a) = (p.polygon_radius().unwrap(), p.polygon_angle().unwrap());
    p.make_sides_equal();
    assert!(p.polygon_sides_equal() && close(p.polygon_radius().unwrap(), r) && close(p.polygon_angle().unwrap(), a));
    let LiveShape::Polygon { xf, .. } = &p else { panic!("polygon") };
    assert_eq!(*xf * Point::ZERO, Point::new(100.0, 50.0));
    // Other shapes have none of it.
    let e = LiveShape::Ellipse { w: 1.0, h: 1.0, pie: (0.0, 360.0), xf: Affine::IDENTITY };
    assert_eq!((e.polygon_radius(), e.polygon_angle()), (None, None));
}

/// A node holding a live rectangle (`w` × `h`, all radii `r`, placed by `xf`) and its path.
fn live_rect(w: f64, h: f64, r: f64, xf: Affine) -> Node {
    let live = LiveShape::Rectangle { w, h, radii: [r; 4], kinds: Default::default(), radii_relative: false, xf };
    let mut n = Node::path(NodeId(1), live.to_path(), Appearance::default_art());
    if let NodeKind::Path { live: slot, .. } = &mut n.kind {
        *slot = Some(Box::new(live));
    }
    n
}

fn live_of(n: &Node) -> (f64, f64, [f64; 4], Affine) {
    let NodeKind::Path { live: Some(l), .. } = &n.kind else { panic!("not a live shape") };
    let LiveShape::Rectangle { w, h, radii, xf, .. } = &**l else { panic!("not a live rectangle") };
    (*w, *h, *radii, *xf)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn near_xf(a: Affine, b: Affine) -> bool {
    a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| near(*x, y))
}

/// Size and radius of a live rectangle, within rounding.
fn assert_live(n: &Node, w: f64, h: f64, r: f64) {
    let (lw, lh, radii, _) = live_of(n);
    assert!(near(lw, w) && near(lh, h) && radii.iter().all(|x| near(*x, r)), "{lw} × {lh} r {radii:?}, want {w} × {h} r {r}");
}

/// Each corner arc of a rounded rectangle's path, mapped by `back` (to undo a rotation),
/// spans `r` along x and y: the corner is a circular arc of radius `r`.
fn assert_round_corners(n: &Node, back: Affine, r: f64) {
    let NodeKind::Path { path, .. } = &n.kind else { panic!("not a path") };
    let a: Vec<Point> = path.subpaths[0].anchors.iter().map(|a| back * a.p).collect();
    assert_eq!(a.len(), 8, "a rounded rectangle");
    // Anchors 1→2, 3→4, 5→6 and 7→0 are the arcs.
    for (i, j) in [(1, 2), (3, 4), (5, 6), (7, 0)] {
        let (x, y) = ((a[j].x - a[i].x).abs(), (a[j].y - a[i].y).abs());
        assert!(near(x, r) && near(y, r), "corner spans {x} × {y}, want {r} × {r}");
    }
}

const KEEP_CORNERS: Scaling = Scaling { strokes: false, effects: None, keep_type_strokes: false, keep_corners: true, patterns: false };

#[test]
fn uneven_scales_keep_live_corners_circular() {
    // A 100 pt rounded square at (10, 10) stretched to 300 × 100 (#291).
    let mut n = live_rect(100.0, 100.0, 20.0, Affine::translate((10.0, 10.0)));
    let stretch = Affine::translate((10.0, 10.0)) * Affine::scale_non_uniform(3.0, 1.0) * Affine::translate((-10.0, -10.0));
    n.transform(stretch, KEEP_CORNERS);
    assert_live(&n, 300.0, 100.0, 20.0);
    assert!(near_xf(live_of(&n).3, Affine::translate((10.0, 10.0))), "the scale went into the size");
    let b = n.geometric_bounds().unwrap();
    assert!(near(b.x0, 10.0) && near(b.y0, 10.0) && near(b.x1, 310.0) && near(b.y1, 110.0), "{b:?}");
    assert_round_corners(&n, Affine::IDENTITY, 20.0);
    // Scale Corners on: the radius scales by the mean scale, still circular.
    let mut n = live_rect(100.0, 100.0, 20.0, Affine::IDENTITY);
    n.transform(Affine::scale_non_uniform(4.0, 1.0), Scaling::default());
    assert_live(&n, 400.0, 100.0, 40.0);
    assert_round_corners(&n, Affine::IDENTITY, 40.0);
    // Squashed below the radius, the short sides are fully round; stretched back, it returns.
    n.transform(Affine::scale_non_uniform(1.0, 0.5), KEEP_CORNERS);
    assert_round_corners(&n, Affine::IDENTITY, 25.0);
    n.transform(Affine::scale_non_uniform(1.0, 2.0), KEEP_CORNERS);
    assert_round_corners(&n, Affine::IDENTITY, 40.0);
}

#[test]
fn rotated_and_reflected_live_rectangles_scale_along_their_own_sides() {
    let place = Affine::translate((200.0, 100.0)) * Affine::rotate(30f64.to_radians());
    let mut n = live_rect(100.0, 50.0, 10.0, place);
    // Stretched along its own width, as a bounding-box drag of a rotated shape does.
    n.transform(place * Affine::scale_non_uniform(2.0, 1.0) * place.inverse(), KEEP_CORNERS);
    assert_live(&n, 200.0, 50.0, 10.0);
    assert!(near_xf(live_of(&n).3, place));
    assert_round_corners(&n, place.inverse(), 10.0);
    // A reflection stays in `xf`; the corners stay as they were.
    n.transform(Affine::scale_non_uniform(-1.0, 1.0), KEEP_CORNERS);
    assert_live(&n, 200.0, 50.0, 10.0);
    let xf = live_of(&n).3;
    assert!(near(xf.determinant(), -1.0));
    assert_round_corners(&n, xf.inverse(), 10.0);
}

#[test]
fn shears_and_moves_keep_the_old_transform() {
    // Scaling a rotated rectangle along the page axes shears it: `xf` takes it all, as before.
    let rot = Affine::rotate(30f64.to_radians());
    let mut n = live_rect(100.0, 50.0, 10.0, rot);
    let before = n.clone();
    let a = Affine::scale_non_uniform(2.0, 1.0);
    n.transform(a, Scaling::default());
    assert_eq!(live_of(&n), (100.0, 50.0, [10.0; 4], a * rot));
    let (NodeKind::Path { path, .. }, NodeKind::Path { path: old, .. }) = (&n.kind, &before.kind) else { panic!() };
    assert_eq!(*path, old.transformed(a));
    // A file saved with an uneven scale in `xf` keeps its exact geometry when moved...
    let legacy = Affine::translate((5.0, 5.0)) * Affine::scale_non_uniform(2.0, 1.0);
    let mut n = live_rect(100.0, 100.0, 20.0, legacy);
    n.transform(Affine::translate((10.0, 0.0)), KEEP_CORNERS);
    assert_eq!(live_of(&n), (100.0, 100.0, [20.0; 4], Affine::translate((10.0, 0.0)) * legacy));
    // ...and once scaled, has document units and circular corners of its mean radius.
    n.transform(Affine::scale(2.0), Scaling::default());
    assert_live(&n, 400.0, 200.0, 20.0 * 2f64.sqrt() * 2.0);
    assert!(near(live_of(&n).3.determinant(), 1.0));
    assert_round_corners(&n, Affine::IDENTITY, 20.0 * 2f64.sqrt() * 2.0);
}

/// A rectangle saved with an uneven scale in `xf` (before #291) has elliptical corners; Live
/// Corners see and edit it in document units, with circular corners (#442).
#[test]
fn a_scale_left_in_the_transform_folds_into_the_size() {
    let legacy = Affine::translate((5.0, 5.0)) * Affine::scale_non_uniform(4.0, 1.0);
    let n = live_rect(50.0, 80.0, 20.0, legacy);
    let NodeKind::Path { live: Some(live), .. } = &n.kind else { panic!("not live") };
    let folded = live.folded();
    assert_eq!(live_of(&n).3, legacy, "folded() leaves the shape alone");
    let LiveShape::Rectangle { w, h, radii, xf, .. } = &folded else { panic!("not a rectangle") };
    // Drawn 200 × 80 with 80 × 20 corners; folded, the corners are circles of the mean radius.
    assert!(near(*w, 200.0) && near(*h, 80.0) && radii.iter().all(|r| near(*r, 40.0)), "{w} × {h} r {radii:?}");
    assert!(near_xf(*xf, Affine::translate((5.0, 5.0))));
    let (a, b) = (folded.to_path().bounds().unwrap(), live.to_path().bounds().unwrap());
    assert!(near(a.x0, b.x0) && near(a.y0, b.y0) && near(a.x1, b.x1) && near(a.y1, b.y1), "same place and size: {a:?} {b:?}");
    let corners = crate::LiveCorners::of(&n).unwrap();
    assert_eq!(corners.style(&corners.all()).0, Some(40.0), "the radius in document units");
    // Nothing to fold: a moved, rotated or reflected rectangle and a sheared one stay as they are.
    let place = Affine::translate((5.0, 5.0)) * Affine::rotate(0.5) * Affine::scale_non_uniform(-1.0, 1.0);
    let shear = Affine::new([1.0, 0.0, 0.5, 1.0, 0.0, 0.0]);
    for xf in [place, shear] {
        let mut l = live_rect(100.0, 160.0, 20.0, xf);
        let NodeKind::Path { live: Some(live), .. } = &mut l.kind else { panic!("not live") };
        assert!(!live.fold_scale());
        assert_eq!(live_of(&l), (100.0, 160.0, [20.0; 4], xf));
    }
}

#[test]
fn default_transparency() {
    let p = || Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    assert!(p().has_default_transparency());
    let tweaks: [fn(&mut Node); 7] = [
        |n| n.opacity = 0.5,
        |n| n.blend = BlendMode::Multiply,
        |n| n.isolate = true,
        |n| n.knockout = Knockout::On,
        |n| n.knockout = Knockout::Off,
        |n| n.knockout_shape = true,
        |n| n.extras_mut().mask = Some(Box::new(OpacityMask::new(n.clone(), true))),
    ];
    for tweak in tweaks {
        let mut n = p();
        tweak(&mut n);
        assert!(!n.has_default_transparency(), "{n:?}");
    }
}

#[test]
fn transform_group_recurses() {
    let p = Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    let mut g = Node::group(NodeId(1), vec![Arc::new(p)]);
    g.transform(Affine::scale(2.0), true);
    assert_eq!(g.geometric_bounds(), Some(Rect::new(0.0, 0.0, 20.0, 20.0)));
    let child = &g.children().unwrap()[0];
    assert_eq!(child.appearance.stroke_width(), 2.0);
}

#[test]
fn unplaced_gradients_pin_only_when_a_refit_would_differ() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
    let grad = || Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let fresh = || Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)), Appearance::basic(grad(), grad(), 2.0));
    let geoms = |n: &Node| match (n.appearance.fill_paint(), n.appearance.stroke_paint()) {
        (Paint::Gradient(f), Paint::Gradient(s)) => (f.geom, s.geom),
        _ => panic!("gradient fill and stroke"),
    };
    let mut n = fresh();
    n.transform(Affine::translate((10.0, 5.0)) * Affine::scale(2.0), true);
    assert_eq!(geoms(&n), (None, None), "moves and uniform scales keep refitting");
    let mut n = fresh();
    n.transform(Affine::rotate(std::f64::consts::FRAC_PI_2), false);
    let (f, s) = geoms(&n);
    // Fitted on the old bounds (strokes on the stroke-inflated box), then rotated: vertical.
    let mut want = GradientGeom::fit(GradientKind::Linear, Rect::new(0.0, 0.0, 100.0, 50.0), 0.0);
    want.transform(Affine::rotate(std::f64::consts::FRAC_PI_2), GradientKind::Linear);
    let f = f.unwrap();
    assert!(f.start.distance(want.start) < 1e-9 && f.end.distance(want.end) < 1e-9, "{f:?}");
    assert!((s.unwrap().length() - 102.0).abs() < 1e-9);
}

#[test]
fn display_names() {
    let p = Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    assert_eq!(p.display_name(), "<Path>");
    let l = Node::layer(NodeId(1), "Layer 1", LayerColor::Preset(0));
    assert_eq!(l.display_name(), "Layer 1");
}

#[test]
fn visual_bounds_include_stroke() {
    let mut p = Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    p.appearance.stroke_mut().unwrap().join = crate::appearance::LineJoin::Round;
    assert_eq!(p.visual_bounds(), Some(Rect::new(-0.5, -0.5, 10.5, 10.5)));
}

/// Type's visual bounds grow by how far its character strokes paint past the glyphs: half the
/// weight centred, the whole weight outside, nothing inside; scaled with the type.
#[test]
fn type_visual_bounds_grow_with_the_character_stroke() {
    use crate::appearance::StrokeAlign;
    use crate::text::{CharStyle, TextObject};
    let style = CharStyle { stroke: vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), stroke_width: 4.0, ..Default::default() };
    let mut t = TextObject::point(vectorcraft_geom::Point::new(0.0, 0.0), "Ab", style);
    t.xf = Affine::scale(2.0) * t.xf;
    let node = |t: &TextObject| Node::new(NodeId(1), NodeKind::Text(Box::new(t.clone())));
    let g = node(&t).geometric_bounds().unwrap();
    for (align, grow) in [(StrokeAlign::Center, 4.0), (StrokeAlign::Outside, 8.0), (StrokeAlign::Inside, 0.0)] {
        t.runs[0].style.stroke_align = align;
        assert_eq!(node(&t).visual_bounds(), Some(g.inflate(grow, grow)), "{align:?}");
    }
    t.runs[0].style.stroke = vectorcraft_color::Paint::None;
    assert_eq!(node(&t).visual_bounds(), Some(g), "no stroke, no growth");
}

/// A group holding a path, a compound, an image and a nested group, as a file writes it.
fn nested_group() -> Node {
    let path = |id| Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    let image = ImageObject { key: "k".into(), width: 2, height: 3, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    let inner = Node::new(NodeId(5), NodeKind::Group { children: vec![Arc::new(path(6))], clip: true });
    let compound = Node::new(NodeId(3), NodeKind::Compound { children: vec![Arc::new(path(4))], rule: FillRule::EvenOdd });
    let kids = vec![Arc::new(path(2)), Arc::new(compound), Arc::new(Node::new(NodeId(7), NodeKind::Image(Box::new(image)))), Arc::new(inner)];
    Node::new(NodeId(1), NodeKind::Group { children: kids, clip: false })
}

#[test]
fn kinds_read_back_with_type_first_or_anywhere() {
    let n = nested_group();
    let json = serde_json::to_string(&n).unwrap();
    assert!(json.contains(r#""kind":{"type":"group","children":["#), "written with `type` first: {json}");
    assert_eq!(serde_json::from_str::<Node>(&json).unwrap(), n);
    // Through a JSON value, whose keys are sorted: `type` comes after `children`.
    let v = serde_json::to_value(&n).unwrap();
    assert_eq!(serde_json::from_value::<Node>(v.clone()).unwrap(), n);
    assert_eq!(serde_json::from_str::<Node>(&v.to_string()).unwrap(), n);
    // `type` last, in the middle, and keys a newer version added (ignored, as before).
    let kind = r#"{"children":[],"x":{"deep":[1,2]},"clip":true,"type":"group"}"#;
    assert_eq!(serde_json::from_str::<NodeKind>(kind).unwrap(), NodeKind::Group { children: vec![], clip: true });
    let kind = r#"{"symbol":"S","type":"symbolinstance","later":1,"xf":[1,0,0,1,5,6]}"#;
    assert_eq!(serde_json::from_str::<NodeKind>(kind).unwrap(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((5.0, 6.0)) });
    let kind = r#"{"width":2,"type":"image","key":"k","height":3,"xf":[1,0,0,1,0,0]}"#;
    assert!(matches!(serde_json::from_str::<NodeKind>(kind).unwrap(), NodeKind::Image(i) if i.key == "k" && i.height == 3));
    // Defaults as before.
    let NodeKind::Layer { printable, preview, template, .. } =
        serde_json::from_str(r#"{"type":"layer","color":{"Preset":0},"children":[]}"#).unwrap()
    else {
        panic!("a layer")
    };
    assert!(printable && preview && !template);
}

#[test]
fn bad_kinds_are_errors() {
    for bad in [r#"{"children":[]}"#, r#"{"type":"nothing"}"#, r#"{"type":"group"}"#, r#"{"type":3,"children":[]}"#, r#"[]"#, r#""group""#] {
        assert!(serde_json::from_str::<NodeKind>(bad).is_err(), "{bad}");
    }
}

/// #3587: [`with_node_budget`] counts every object read, nested ones included, refuses the one past
/// it with a message naming the budget, and is gone once its reading is done.
#[test]
fn objects_are_read_within_the_node_budget() {
    let group = Node::group(NodeId(1), vec![Arc::new(Node::group(NodeId(2), vec![])), Arc::new(Node::group(NodeId(3), vec![]))]);
    let json = serde_json::to_string(&group).unwrap();
    let read = || serde_json::from_str::<Node>(&json);
    assert_eq!(with_node_budget(3, read).unwrap(), group, "three objects fit a budget of three");
    let why = with_node_budget(2, read).unwrap_err().to_string();
    assert!(why.contains("more than 2 objects"), "{why}");
    assert_eq!(read().unwrap(), group, "no budget outside a read");
    // A budget left by a read that failed doesn't carry over to the next one.
    assert!(with_node_budget(1, read).is_err());
    assert!(with_node_budget(3, read).is_ok());
}

/// #3587: fields read before an object's `type` are buffered value by value within
/// [`with_buffered_value_budget`], each counted once: a group read back from the buffer buffers
/// its own fields again without counting them twice. VectorCraft writes `type` first, which
/// buffers nothing.
#[test]
fn fields_before_the_type_are_buffered_within_the_value_budget() {
    let reordered = r#"{"id":1,"kind":{"children":[{"id":2,"kind":{"children":[],"type":"group"}}],"type":"group"}}"#;
    let read = |json: &str| serde_json::from_str::<Node>(json);
    // The children's array, the inner object, its id, its kind, the kind's empty children and
    // its type: six values.
    assert!(with_buffered_value_budget(6, || read(reordered)).is_ok());
    let why = with_buffered_value_budget(5, || read(reordered)).unwrap_err().to_string();
    assert!(why.contains("more than 5 values before its type"), "{why}");
    let written = serde_json::to_string(&Node::group(NodeId(1), vec![Arc::new(Node::group(NodeId(2), vec![]))])).unwrap();
    assert!(with_buffered_value_budget(0, || read(&written)).is_ok(), "a saved object buffers nothing: {written}");
    assert!(read(reordered).is_ok(), "no budget outside a read");
}
