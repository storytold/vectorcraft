use super::*;
use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{CharStyle, Effect, NodeId, TextObject};
use vectorcraft_geom::{Point, Rect, Shape, shapes};

/// Type tests need glyph outlines, which only the optional craft-fonts build input provides.
fn skip_without_craft_fonts() -> bool {
    let skip = vectorcraft_text::CRAFT_FONTS.is_empty();
    if skip {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
    }
    skip
}

fn fx(id: &str, params: serde_json::Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

/// Asserts that placed gradient `got` has `want`'s start, end, aspect and focal point, within 1e-6.
fn assert_geom_near(got: vectorcraft_color::GradientGeom, want: vectorcraft_color::GradientGeom, what: &str) {
    let near = |a: Point, b: Point| a.distance(b) < 1e-6;
    let focal = match (got.focal, want.focal) {
        (None, None) => true,
        (Some(a), Some(b)) => near(a, b),
        _ => false,
    };
    assert!(
        near(got.start, want.start) && near(got.end, want.end) && (got.aspect - want.aspect).abs() < 1e-6 && focal,
        "{what}: got {got:?}, want {want:?}"
    );
}

fn doc_with(n: Node) -> Document {
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

#[test]
fn plain_documents_are_not_copied() {
    let mut d = Document::new(100.0, 100.0);
    let id = d.alloc_id();
    let d = doc_with(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art()));
    assert!(bake_document(&d).is_none());
}

#[test]
fn object_effects_become_geometry_and_raster_effects_stay() {
    let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
    n.appearance.effects.push(fx("path.offsetPath", json!({"offset": 10.0})));
    n.appearance.effects.push(fx("stylize.dropShadow", json!({})));
    let d = bake_document(&doc_with(n)).unwrap();
    let m = d.node(NodeId(50)).unwrap();
    let b = m.geometric_bounds().unwrap();
    assert!((b.width() - 120.0).abs() < 0.5, "{b:?}");
    assert_eq!(m.appearance.effects.len(), 1);
    assert_eq!(m.appearance.effects[0].id, "stylize.dropShadow");
}

/// The fill colors of the paths under the layers of `d`, in paint order.
fn path_fills(d: &Document) -> Vec<Color> {
    let mut out = vec![];
    d.walk(|n| {
        if matches!(n.kind, NodeKind::Path { .. })
            && let Some(c) = n.appearance.fill_paint().color()
        {
            out.push(c);
        }
    });
    out
}

/// A symbol instance half stained red is baked in its stained colors, as the canvas draws it:
/// moved by a Transform effect, and recolored by a color adjustment (Invert Colors) after the
/// stain.
#[test]
fn a_stained_instance_is_baked_in_its_stained_colors() {
    let blue = Color::rgb8(30, 90, 220);
    let art = Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(Paint::solid(blue), Paint::None, 0.0));
    let mut inst = Node::new(NodeId(50), NodeKind::SymbolInstance { symbol: "Mark".into(), xf: vectorcraft_geom::Affine::translate((20.0, 20.0)) });
    inst.appearance = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0).into();
    inst.appearance.items.retain(|i| i.is_fill());
    if let vectorcraft_doc::AppearanceItem::Fill(f) = &mut inst.appearance.items[0] {
        f.opacity = 0.5;
    }
    let stained = vectorcraft_doc::symbol_overrides::instance_art(&art, &inst).appearance.fill_paint().color().unwrap();
    assert_ne!(stained.to_hex(), blue.to_hex());
    let [r, g, b] = stained.to_rgb();
    let inverted = Color::rgb(1.0 - r, 1.0 - g, 1.0 - b);
    for (id, params, want) in [("distort.transform", json!({"moveH": 30.0}), stained), (crate::addons::color::INVERT, json!({}), inverted)] {
        let mut n = inst.clone();
        n.appearance.effects.push(fx(id, params));
        let mut d = doc_with(n);
        d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Mark".into(), art: Arc::new(art.clone()) });
        let fills = path_fills(&bake_document(&d).unwrap());
        assert_eq!(fills.iter().map(Color::to_hex).collect::<Vec<_>>(), [want.to_hex()], "{id}");
    }
}

#[test]
fn a_baked_transform_moves_a_placed_gradient_with_the_path() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: Point::new(0.0, 50.0), end: Point::new(100.0, 50.0), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let placed_geom = |n: &Node| match n.appearance.fill_paint() {
        Paint::Gradient(g) => g.geom.unwrap(),
        p => panic!("a gradient: {p:?}"),
    };
    // A move 30 pt right, and a turn of 90° with the width halved about the center (50, 50): the
    // square becomes 100 pt wide and 50 pt tall, and the gradient runs from the bottom up.
    let moves = [
        (json!({"moveH": 30.0}), Point::new(30.0, 0.0), GradientGeom { start: Point::new(30.0, 50.0), end: Point::new(130.0, 50.0), ..geom }),
        (
            json!({"rotate": 90.0, "scaleH": 50.0}),
            Point::new(0.0, 25.0),
            GradientGeom { start: Point::new(50.0, 75.0), end: Point::new(50.0, 25.0), ..geom },
        ),
    ];
    // Alone, with a stroke whose own effect makes one path for each fill and stroke, and as the
    // fill's own effect.
    for (params, corner, want) in moves {
        for (split, own) in [(false, false), (true, false), (true, true)] {
            let mut ap = Appearance::basic(placed.clone(), Paint::solid(Color::BLACK), if split { 1.0 } else { 0.0 });
            if split && let AppearanceItem::Stroke(s) = &mut ap.items[1] {
                s.effects.push(fx("path.offsetPath", json!({"offset": 5.0})));
            }
            let transform = fx("distort.transform", params.clone());
            if own {
                ap.items[0].effects_mut().push(transform.clone());
            }
            let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), ap);
            if !own {
                n.appearance.effects.push(transform);
            }
            let d = bake_document(&doc_with(n)).unwrap();
            let m = d.node(NodeId(50)).unwrap();
            let fill: &Node = if split { &m.children().unwrap()[0] } else { m };
            let b = fill.geometric_bounds().unwrap();
            assert!(b.origin().distance(corner) < 1e-6, "{params}, split {split}, own {own}: {b:?}");
            assert_geom_near(placed_geom(fill), want, &format!("{params}, split {split}, own {own}"));
        }
    }
}

/// A stroke's own Transform effect or Free Distort moves the gradient placed on that stroke.
#[test]
fn a_strokes_own_transform_and_free_distort_move_its_placed_gradient() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: Point::new(0.0, 50.0), end: Point::new(100.0, 50.0), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    // Moved 30 pt right by Transform, and one width (100 pt) right by Free Distort.
    let moves = [
        (fx("distort.transform", json!({"moveH": 30.0})), 30.0),
        (fx("distort.freeDistort", json!({"corners": [[1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0]]})), 100.0),
    ];
    for (e, moved) in moves {
        let mut ap = Appearance::basic(Paint::None, placed.clone(), 4.0);
        let Some(stroke) = ap.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) else { panic!("a stroke") };
        stroke.effects_mut().push(e.clone());
        let n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), ap);
        let d = bake_document(&doc_with(n)).unwrap();
        let mut geoms = vec![];
        d.node(NodeId(50)).unwrap().walk(&mut |c: &Node| {
            for item in &c.appearance.items {
                if let AppearanceItem::Stroke(s) = item
                    && let Paint::Gradient(g) = &s.paint
                {
                    geoms.push(g.geom.unwrap());
                }
            }
        });
        assert!(!geoms.is_empty(), "{}: the stroke keeps its gradient", e.id);
        let want = GradientGeom { start: Point::new(moved, 50.0), end: Point::new(100.0 + moved, 50.0), ..geom };
        for g in geoms {
            assert_geom_near(g, want, &e.id);
        }
    }
}

#[test]
fn a_fills_gradient_follows_the_effects_around_its_transform_with_options() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: Point::new(0.0, 50.0), end: Point::new(100.0, 50.0), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let moved = fx("distort.transform", json!({"moveH": 30.0}));
    let scales_strokes = fx("distort.transform", json!({"strokes": true}));
    // A move before and a move after the fill's Transform effect with Scale Strokes & Effects on.
    for fill_fx in [vec![moved.clone(), scales_strokes.clone()], vec![scales_strokes, moved]] {
        let mut ap = Appearance::basic(placed.clone(), Paint::None, 0.0);
        *ap.items[0].effects_mut() = fill_fx.clone();
        let n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), ap);
        let d = bake_document(&doc_with(n)).unwrap();
        let mut found = vec![];
        d.node(NodeId(50)).unwrap().walk(&mut |c: &Node| {
            if let Paint::Gradient(g) = c.appearance.fill_paint() {
                found.push((c.geometric_bounds().unwrap(), g.geom.unwrap()));
            }
        });
        assert!(!found.is_empty(), "{fill_fx:?}: the fill keeps its gradient");
        let want = GradientGeom { start: Point::new(30.0, 50.0), end: Point::new(130.0, 50.0), ..geom };
        for (b, g) in found {
            assert!(b.origin().distance(Point::new(30.0, 0.0)) < 1e-6, "{fill_fx:?}: {b:?}");
            assert_geom_near(g, want, &format!("{fill_fx:?}"));
        }
    }
}

#[test]
fn baked_free_distort_moves_placed_gradients_from_the_bounds_to_the_distorted_bounds() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = |kind| GradientPaint::new(Gradient::new(kind, stops.clone()));
    let linear = GradientGeom { start: Point::new(20.0, 50.0), end: Point::new(80.0, 50.0), aspect: 1.0, focal: None };
    let radial = GradientGeom { start: Point::new(40.0, 30.0), end: Point::new(60.0, 30.0), aspect: 0.5, focal: Some(Point::new(45.0, 32.0)) };
    let placed_linear = Paint::Gradient(Box::new(GradientPaint { geom: Some(linear), ..ramp(GradientKind::Linear) }));
    let placed_radial = Paint::Gradient(Box::new(GradientPaint { geom: Some(radial), ..ramp(GradientKind::Radial) }));
    let fitted = Paint::Gradient(Box::new(ramp(GradientKind::Linear)));
    let baked = |fill: &Paint, e: Effect| {
        let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(fill.clone(), Paint::None, 0.0));
        n.appearance.effects.push(e);
        let d = bake_document(&doc_with(n)).unwrap();
        let m = d.node(NodeId(50)).unwrap();
        match m.appearance.fill_paint() {
            Paint::Gradient(g) => (m.geometric_bounds().unwrap(), (*g).clone()),
            p => panic!("a gradient: {p:?}"),
        }
    };
    let near = |a: Point, b: Point| a.distance(b) < 1e-6;
    // The bottom right corner moves 50 pt right, so the bounds grow from 0..100 to 0..150 across.
    let corners = json!({"corners": [[0.0, 0.0], [1.0, 0.0], [1.5, 1.0], [0.0, 1.0]]});
    let distort = || fx("distort.freeDistort", corners.clone());
    let (b, g) = baked(&placed_linear, distort());
    assert!((b.x1 - 150.0).abs() < 1e-6, "{b:?}");
    let geom = g.geom.unwrap();
    assert!(near(geom.start, Point::new(30.0, 50.0)) && near(geom.end, Point::new(120.0, 50.0)), "{geom:?}");
    // A radial gradient's center, focal point and ellipse take the same stretch: the radius along
    // the vector grows from 20 to 30 pt and the 10 pt radius across it stays.
    let (_, g) = baked(&placed_radial, distort());
    let want = GradientGeom { start: Point::new(60.0, 30.0), end: Point::new(90.0, 30.0), aspect: 1.0 / 3.0, focal: Some(Point::new(67.5, 32.0)) };
    assert_geom_near(g.geom.unwrap(), want, "radial");
    // A fitted gradient stays fitted, and fits the distorted outline.
    assert_eq!(baked(&fitted, distort()).1.geom, None);
}

/// A gradient fitted to the object turns, reflects and stretches with a Transform effect that a
/// refit would not reproduce: it is fixed to its fit on the geometry the effect takes, then mapped.
/// Moves and even scales leave it fitted.
#[test]
fn a_fitted_gradient_turns_reflects_and_stretches_with_a_transform_effect() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let fitted = |kind| Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(kind, stops.clone()))));
    let geom = |start: (f64, f64), end: (f64, f64)| GradientGeom { start: start.into(), end: end.into(), aspect: 1.0, focal: None };
    // The geometry of the baked fill (`own`: the Transform effect is the fill's own effect).
    let baked = |kind, effects: &[Effect], own: bool| {
        let mut ap = Appearance::basic(fitted(kind), Paint::None, 0.0);
        if own {
            *ap.items[0].effects_mut() = effects.to_vec();
        }
        let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), ap);
        if !own {
            n.appearance.effects = effects.to_vec();
        }
        let d = bake_document(&doc_with(n)).unwrap();
        let mut found = vec![];
        d.node(NodeId(50)).unwrap().walk(&mut |c: &Node| {
            if let Paint::Gradient(g) = c.appearance.fill_paint() {
                found.push(g.geom);
            }
        });
        assert_eq!(found.len(), 1, "{effects:?}, own {own}");
        found[0]
    };
    let t = |params| fx("distort.transform", params);
    let offset = fx("path.offsetPath", json!({"offset": 10.0, "joins": "miter"}));
    let corners = json!({"corners": [[1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0]]});
    let linear = GradientKind::Linear;
    let cases = [
        // A quarter turn about the center: the left to right ramp runs from the bottom up.
        (linear, vec![t(json!({"rotate": 90.0}))], geom((50.0, 100.0), (50.0, 0.0))),
        // Reflected left to right: the ramp runs from right to left.
        (linear, vec![t(json!({"reflectX": true}))], geom((100.0, 50.0), (0.0, 50.0))),
        // With Scale Strokes & Effects on, the same.
        (linear, vec![t(json!({"rotate": 90.0, "strokes": true}))], geom((50.0, 100.0), (50.0, 0.0))),
        // Offset Path grows the square to -10..110 first: the turn takes the fit across 120 pt.
        (linear, vec![offset.clone(), t(json!({"rotate": 90.0}))], geom((50.0, 110.0), (50.0, -10.0))),
        // Stretched to twice the width: the circle of radius 50 about the center becomes an
        // ellipse 100 pt across and 50 pt tall.
        (GradientKind::Radial, vec![t(json!({"scaleH": 200.0}))], GradientGeom { aspect: 0.5, ..geom((50.0, 50.0), (150.0, 50.0)) }),
        // Turned, then moved one width right by Free Distort: the fixed gradient moves with it.
        (linear, vec![t(json!({"rotate": 90.0})), fx("distort.freeDistort", corners)], geom((150.0, 100.0), (150.0, 0.0))),
    ];
    for (kind, effects, want) in cases {
        for own in [false, true] {
            let what = format!("{kind:?} {effects:?}, own {own}");
            let got = baked(kind, &effects, own).unwrap_or_else(|| panic!("{what}: fixed to its fit"));
            assert_geom_near(got, want, &what);
        }
    }
    // A move and an even scale are what a refit gives: the gradient stays fitted.
    for effects in [vec![t(json!({"moveH": 30.0}))], vec![t(json!({"scaleH": 150.0, "scaleV": 150.0})), offset]] {
        for own in [false, true] {
            assert_eq!(baked(linear, &effects, own), None, "{effects:?}, own {own}");
        }
    }
    // A stroke's gradient is fixed to its fit on the box grown by half its weight (4 pt), -2..102,
    // with Scale Strokes & Effects off or on.
    let stroked = |effects: &[Effect], own: bool| {
        let mut ap = Appearance::basic(Paint::None, fitted(linear), 4.0);
        let Some(stroke) = ap.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) else { panic!("a stroke") };
        if own {
            *stroke.effects_mut() = effects.to_vec();
        }
        let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), ap);
        if !own {
            n.appearance.effects = effects.to_vec();
        }
        let d = bake_document(&doc_with(n)).unwrap();
        let mut found = vec![];
        d.node(NodeId(50)).unwrap().walk(&mut |c: &Node| {
            for item in &c.appearance.items {
                if let AppearanceItem::Stroke(s) = item
                    && let Paint::Gradient(g) = &s.paint
                {
                    found.push(g.geom);
                }
            }
        });
        assert_eq!(found.len(), 1, "{effects:?}, own {own}");
        found[0]
    };
    for strokes in [false, true] {
        for own in [false, true] {
            let what = format!("stroke, strokes {strokes}, own {own}");
            let got = stroked(&[t(json!({"rotate": 90.0, "strokes": strokes}))], own).unwrap_or_else(|| panic!("{what}: fixed to its fit"));
            assert_geom_near(got, geom((50.0, 102.0), (50.0, -2.0)), &what);
        }
    }
}

/// A Transform effect that scales the geometry to nothing across leaves a fitted gradient fitted:
/// mapping it with the effect would flatten it to one color.
#[test]
fn a_transform_effect_that_scales_to_nothing_leaves_a_fitted_gradient_fitted() {
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let fitted = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    // With Scale Strokes & Effects or Transform Patterns on, and for each copy, as without them.
    for (options, strokes) in [
        (json!({}), 1),
        (json!({"strokes": true}), 1),
        (json!({"patterns": true}), 1),
        (json!({"copies": 2}), 3),
        (json!({"strokes": true, "copies": 2}), 3),
        (json!({"patterns": true, "copies": 2}), 3),
    ] {
        let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::None, fitted.clone(), 4.0));
        let mut p = json!({"scaleH": 0.0, "rotate": 30.0});
        if let (Some(p), Some(o)) = (p.as_object_mut(), options.as_object()) {
            p.extend(o.clone());
        }
        n.appearance.effects = vec![fx("distort.transform", p)];
        let d = bake_document(&doc_with(n)).unwrap();
        let mut found = vec![];
        d.node(NodeId(50)).unwrap().walk(&mut |c: &Node| {
            for item in &c.appearance.items {
                if let AppearanceItem::Stroke(s) = item
                    && let Paint::Gradient(g) = &s.paint
                {
                    found.push(g.geom);
                }
            }
        });
        assert_eq!(found, vec![None; strokes], "{options}");
    }
}

#[test]
fn baked_warps_move_placed_gradients_from_the_bounds_to_the_warped_bounds() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = |kind| GradientPaint::new(Gradient::new(kind, stops.clone()));
    // A ramp across the square and one down it, so both axes of the box map show.
    let across = GradientGeom { start: Point::new(20.0, 30.0), end: Point::new(80.0, 30.0), aspect: 1.0, focal: None };
    let down = GradientGeom { start: Point::new(60.0, 10.0), end: Point::new(60.0, 75.0), aspect: 1.0, focal: None };
    let placed = |geom| Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp(GradientKind::Linear) }));
    let fitted = Paint::Gradient(Box::new(ramp(GradientKind::Linear)));
    let square = Rect::new(0.0, 0.0, 100.0, 100.0);
    let baked = |fill: &Paint, e: Effect| {
        let mut n = Node::path(NodeId(50), shapes::rectangle(square), Appearance::basic(fill.clone(), Paint::None, 0.0));
        n.appearance.effects.push(e);
        let d = bake_document(&doc_with(n)).unwrap();
        let m = d.node(NodeId(50)).unwrap();
        match m.appearance.fill_paint() {
            Paint::Gradient(g) => (m.geometric_bounds().unwrap(), (*g).clone()),
            p => panic!("a gradient: {p:?}"),
        }
    };
    // Where the box map from the square to `to` takes `p`.
    let follow = |p: Point, to: Rect| Point::new(to.x0 + p.x / 100.0 * to.width(), to.y0 + p.y / 100.0 * to.height());
    let styles = [
        "arc",
        "arcLower",
        "arcUpper",
        "arch",
        "bulge",
        "shellLower",
        "shellUpper",
        "flag",
        "wave",
        "fish",
        "rise",
        "fisheye",
        "inflate",
        "squeeze",
        "twist",
    ];
    // The square's warped bounds, from `warp_point` on 401 points along each side: the square's
    // corners are (-1, -1) and (1, 1) in the warp's box coordinates, and a vertical warp swaps
    // the axes.
    let warped_bounds = |style: &str, p: &serde_json::Value| {
        let Some(style) = crate::WarpStyle::from_id(style) else { panic!("warp.{style}: a style") };
        let share = |key: &str, default: f64| p.get(key).and_then(serde_json::Value::as_f64).unwrap_or(default) / 100.0;
        let vertical = p.get("orientation").and_then(serde_json::Value::as_str) == Some("vertical");
        let (bend, dh, dv) = (share("bend", 50.0), share("horizontal", 0.0), share("vertical", 0.0));
        let mut r: Option<Rect> = None;
        for k in 0..=400 {
            let s = f64::from(k) / 200.0 - 1.0;
            for (x, y) in [(s, -1.0), (s, 1.0), (-1.0, s), (1.0, s)] {
                let (x2, y2) = if vertical {
                    let (a, b) = crate::warp_point(style, bend, dh, dv, 1.0, y, x);
                    (b, a)
                } else {
                    crate::warp_point(style, bend, dh, dv, 1.0, x, y)
                };
                let q = Point::new(50.0 + x2 * 50.0, 50.0 + y2 * 50.0);
                r = Some(r.map_or(Rect::from_points(q, q), |r| r.union_pt(q)));
            }
        }
        r.unwrap_or(square)
    };
    let near =
        |a: Rect, b: Rect, tol: f64| (a.x0 - b.x0).abs() < tol && (a.y0 - b.y0).abs() < tol && (a.x1 - b.x1).abs() < tol && (a.y1 - b.y1).abs() < tol;
    let mut moved = 0;
    for style in styles {
        for params in
            [json!({"bend": 30.0}), json!({"bend": -40.0, "orientation": "vertical"}), json!({"bend": 20.0, "horizontal": 30.0, "vertical": -20.0})]
        {
            // A warp that bends the square's sides inward keeps its bounds (within 0.5 pt), and the
            // gradient stays with them.
            let expected = warped_bounds(style, &params);
            let moves = !near(expected, square, 0.5);
            moved += usize::from(moves);
            for linear in [across, down] {
                let what = format!("warp.{style} {params} {linear:?}");
                let (b, g) = baked(&placed(linear), fx(&format!("warp.{style}"), params.clone()));
                assert!(near(b, expected, 0.1), "{what}: bounds {b:?}, want {expected:?}");
                let geom = g.geom.unwrap();
                let shifted = geom.start.distance(linear.start).max(geom.end.distance(linear.end));
                assert_eq!(shifted > 0.1, moves, "{what}: the gradient moves {shifted} with bounds {b:?}");
                let want = GradientGeom { start: follow(linear.start, b), end: follow(linear.end, b), ..linear };
                assert_geom_near(geom, want, &what);
            }
            let what = format!("warp.{style} {params}");
            // A fitted gradient stays fitted, and fits the warped outline.
            assert_eq!(baked(&fitted, fx(&format!("warp.{style}"), params)).1.geom, None, "{what}");
        }
    }
    assert_eq!(moved, 39, "the warps that move the bounds, of 45");
    // With no bend and no distortion the gradient stays where it is.
    let (_, g) = baked(&placed(across), fx("warp.arc", json!({"bend": 0.0})));
    assert_geom_near(g.geom.unwrap(), across, "no bend");
}

/// A warp with no distortion and no bend (or a bend Arc takes as none) leaves a placed gradient
/// exactly where it was, even where warping the outline moves its bounds by a rounding error.
#[test]
fn a_warp_with_no_bend_leaves_a_placed_gradient_exactly_in_place() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let geom = GradientGeom { start: Point::new(13.7, 31.3), end: Point::new(91.1, 47.9), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..GradientPaint::new(Gradient::new(GradientKind::Linear, stops)) }));
    let mut drift = 0;
    let cases = crate::WARP_STYLES.map(|(s, _)| (s, 0.0));
    for (k, (style, bend)) in cases.into_iter().chain([("arc", 1e-5), ("arc", -1e-5)]).enumerate() {
        let k = k as f64;
        let oval = Rect::new(10.1 + k * 0.37, 20.3 - k * 1.13, 97.7 + k * 3.31, 71.9 + k * 0.71);
        let mut n = Node::path(NodeId(50), shapes::ellipse(oval), Appearance::basic(placed.clone(), Paint::None, 0.0));
        n.appearance.effects.push(fx(&format!("warp.{style}"), json!({ "bend": bend })));
        let d = bake_document(&doc_with(n)).unwrap();
        let m = d.node(NodeId(50)).unwrap();
        drift += usize::from(m.geometric_bounds() != shapes::ellipse(oval).bounds());
        let Paint::Gradient(g) = m.appearance.fill_paint() else { panic!("warp.{style} {bend}: a gradient") };
        assert_eq!(g.geom, Some(geom), "warp.{style} {bend}");
    }
    assert!(drift > 0, "the warped outlines' bounds move by a rounding error");
    // Another style with as small a bend still moves the outline, and the gradient follows it.
    let mut n = Node::path(NodeId(50), shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(placed, Paint::None, 0.0));
    n.appearance.effects.push(fx("warp.rise", json!({"bend": 1e-5})));
    let d = bake_document(&doc_with(n)).unwrap();
    let Paint::Gradient(g) = d.node(NodeId(50)).unwrap().appearance.fill_paint() else { panic!("warp.rise: a gradient") };
    assert_ne!(g.geom, Some(geom), "warp.rise 1e-5");
}

/// A Warp effect on a fill or a stroke itself moves and scales its placed gradient from the
/// path's bounds before the warp to its bounds after it (a stroke: its stroke-inflated bounds).
#[test]
fn a_fills_and_a_strokes_own_warp_move_its_placed_gradient_with_the_bounds() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: Point::new(20.0, 60.0), end: Point::new(80.0, 60.0), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let square = Rect::new(0.0, 0.0, 100.0, 100.0);
    let width = 4.0;
    // Where the box map from `from` to `to` takes `p`.
    let follow = |p: Point, from: Rect, to: Rect| {
        Point::new(to.x0 + (p.x - from.x0) / from.width() * to.width(), to.y0 + (p.y - from.y0) / from.height() * to.height())
    };
    let pad = |r: Rect| r.inflate(width / 2.0, width / 2.0);
    for style in ["arc", "rise", "inflate"] {
        for stroke in [false, true] {
            let what = format!("warp.{style} on the {}", if stroke { "stroke" } else { "fill" });
            let mut ap =
                if stroke { Appearance::basic(Paint::None, placed.clone(), width) } else { Appearance::basic(placed.clone(), Paint::None, 0.0) };
            let Some(item) = ap.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_)) == stroke) else { panic!("{what}: the item") };
            item.effects_mut().push(fx(&format!("warp.{style}"), json!({"bend": 40.0})));
            let n = Node::path(NodeId(50), shapes::rectangle(square), ap);
            let d = bake_document(&doc_with(n)).unwrap();
            let mut found = vec![];
            d.node(NodeId(50)).unwrap().walk(&mut |c: &Node| {
                for item in &c.appearance.items {
                    if let Paint::Gradient(g) = item.paint() {
                        found.push((c.geometric_bounds().unwrap(), g.geom.unwrap()));
                    }
                }
            });
            assert!(!found.is_empty(), "{what}: the gradient is kept");
            for (b, g) in found {
                assert_ne!(b, square, "{what}: the warp changes the bounds");
                let (from, to) = if stroke { (pad(square), pad(b)) } else { (square, b) };
                let want = GradientGeom { start: follow(geom.start, from, to), end: follow(geom.end, from, to), ..geom };
                assert_geom_near(g, want, &what);
            }
        }
    }
}

#[test]
fn item_effects_split_into_one_path_per_item() {
    let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
    if let AppearanceItem::Stroke(s) = &mut n.appearance.items[1] {
        s.effects.push(fx("path.offsetPath", json!({"offset": 5.0})));
    }
    let d = bake_document(&doc_with(n)).unwrap();
    let m = d.node(NodeId(50)).unwrap();
    let ch = m.children().unwrap();
    assert_eq!(ch.len(), 2);
    assert!((ch[0].geometric_bounds().unwrap().width() - 100.0).abs() < 1e-6);
    assert!((ch[1].geometric_bounds().unwrap().width() - 110.0).abs() < 0.5);
    assert!(ch[0].id != ch[1].id && ch[0].id != NodeId(50));
}

#[test]
fn a_baked_stroke_keeps_its_raster_effects() {
    let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
    if let AppearanceItem::Stroke(s) = &mut n.appearance.items[1] {
        s.effects.push(fx("path.offsetPath", json!({"offset": 5.0})));
        s.effects.push(fx("stylize.dropShadow", json!({})));
    }
    let m = bake_appearance(None, &n).unwrap();
    let ch = m.children().unwrap();
    let Some(AppearanceItem::Fill(f)) = ch[1].appearance.items.first() else { panic!("the outline is filled") };
    assert_eq!(f.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["stylize.dropShadow"]);
}

#[test]
fn a_baked_stroke_leaves_dither_behind() {
    // Strokes ignore dither, so the filled outline goes without it and looks as the stroke did.
    let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
    if let AppearanceItem::Stroke(s) = &mut n.appearance.items[1] {
        let mut g = vectorcraft_color::GradientPaint::new(Default::default());
        g.gradient.dither = true;
        s.paint = Paint::Gradient(Box::new(g));
    }
    let m = bake_appearance(None, &n).unwrap();
    let ch = m.children().unwrap();
    let Some(AppearanceItem::Fill(f)) = ch[1].appearance.items.first() else { panic!("the outline is filled") };
    let Paint::Gradient(g) = &f.paint else { panic!("a gradient fill") };
    assert!(!g.gradient.dither);
}

#[test]
fn a_baked_stroke_with_a_gradient_along_it_becomes_meshes_clipped_to_its_outline() {
    let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
    if let AppearanceItem::Stroke(s) = &mut n.appearance.items[1] {
        s.width = 10.0;
        s.paint = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(Default::default())));
        s.gradient_mode = vectorcraft_doc::StrokeGradientMode::Along;
        s.opacity = 0.5;
    }
    let m = bake_appearance(None, &n).unwrap();
    let ch = m.children().unwrap();
    let NodeKind::Group { children, clip: true } = &ch[1].kind else { panic!("a clip group: {:?}", ch[1].kind) };
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    assert!(children.len() > 1 && children[1..].iter().all(|c| matches!(c.kind, NodeKind::Mesh(_))), "gradient meshes");
    assert_eq!(ch[1].opacity, 0.5);
}

/// Distort Appearance on the pieces of type's glyph outlines (#6644): a stroke's geometry effect
/// still bakes into its piece, but the stroke stays a stroke at its own weight; a piece with no
/// geometry effect is left as it is, its stroke on it (Distort Appearance on a path outlines it).
#[test]
fn type_pieces_bake_their_geometry_effects_and_keep_their_strokes() {
    let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
    assert!(bake_type_appearance(None, &n).is_none(), "nothing to bake");
    assert!(bake_appearance(None, &n).is_some(), "a path's stroke is outlined");
    if let AppearanceItem::Stroke(s) = &mut n.appearance.items[1] {
        s.width = 4.0;
        s.effects.push(fx("path.offsetPath", json!({"offset": 5.0})));
    }
    let m = bake_type_appearance(None, &n).unwrap();
    let ch = m.children().unwrap();
    assert_eq!(ch.len(), 2);
    let Some(AppearanceItem::Stroke(st)) = ch[1].appearance.items.first() else { panic!("still a stroke: {:?}", ch[1].appearance) };
    assert!(st.width == 4.0 && st.effects.is_empty(), "{st:?}");
    assert!((ch[1].geometric_bounds().unwrap().width() - 110.0).abs() < 0.5, "the stroke's offset is baked into its path");
    assert!((ch[0].geometric_bounds().unwrap().width() - 100.0).abs() < 1e-6, "the fill keeps the square");
}

/// A 100 pt square at the origin painted by `fill` and `stroke` (`width` pt), with the Transform
/// effect `params`.
fn transformed(fill: Paint, stroke: Paint, width: f64, params: serde_json::Value) -> Node {
    let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(fill, stroke, width));
    n.appearance.effects.push(fx("distort.transform", params));
    n
}

/// The paths under `n`, in document order.
fn baked_paths(n: &Node) -> Vec<&Node> {
    match n.children() {
        Some(ch) if !matches!(n.kind, NodeKind::Compound { .. }) => ch.iter().flat_map(|c| baked_paths(c)).collect(),
        _ => vec![n],
    }
}

fn widths(n: &Node) -> Vec<f64> {
    baked_paths(n).iter().map(|p| p.geometric_bounds().unwrap().width()).collect()
}

#[test]
fn a_baked_transform_copy_keeps_its_scaled_stroke() {
    // Distort Appearance: the 200% copy's stroke is 4 pt, as outside the envelope, so its outline
    // is 204 pt wide.
    let n = transformed(Paint::None, Paint::solid(Color::BLACK), 2.0, json!({"scaleH": 200, "scaleV": 200, "copies": 1, "strokes": true}));
    let m = bake_appearance(None, &n).unwrap();
    let w = widths(&m);
    assert_eq!(w.len(), 2, "{w:?}");
    assert!((w[1] - 102.0).abs() < 0.5 && (w[0] - 204.0).abs() < 0.5, "the original on top: {w:?}");
    assert!(baked_paths(&m).iter().all(|p| p.appearance.stroke().is_none()), "the strokes are outlined");
}

#[test]
fn a_baked_transform_copy_keeps_its_moved_pattern() {
    let paint = Paint::Pattern { pattern: "p".into(), xf: vectorcraft_geom::Affine::IDENTITY };
    let n = transformed(paint, Paint::None, 0.0, json!({"moveH": 30, "copies": 1, "patterns": true}));
    let m = bake_appearance(None, &n).unwrap();
    let xs: Vec<f64> = baked_paths(&m)
        .iter()
        .map(|p| match p.appearance.fill_paint() {
            Paint::Pattern { xf, .. } => xf.as_coeffs()[4],
            p => panic!("not a pattern: {p:?}"),
        })
        .collect();
    assert_eq!(xs, vec![30.0, 0.0], "the original on top");
}

#[test]
fn a_baked_transform_without_its_options_outlines_the_copies_together() {
    // Without Scale Strokes & Effects the copy keeps the 2 pt stroke: one outline of both squares.
    let n = transformed(Paint::None, Paint::solid(Color::BLACK), 2.0, json!({"scaleH": 200, "scaleV": 200, "copies": 1}));
    let m = bake_appearance(None, &n).unwrap();
    let w = widths(&m);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!((w[0] - 202.0).abs() < 0.5, "{w:?}");
}

/// Black 24 pt "Type" with its baseline at (20, 60).
fn type_node(id: u64) -> Node {
    let style = CharStyle { size: 24.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    Node::new(NodeId(id), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 60.0), "Type", style))))
}

/// The origins of the type objects in `d`, in paint order.
fn type_origins(d: &Document) -> Vec<(f64, f64)> {
    let mut out = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            let c = t.xf.as_coeffs();
            out.push((c[4], c[5]));
        }
    });
    out
}

/// A group of the type and a 10 pt square with `effects`.
fn type_group(effects: Vec<Effect>) -> Node {
    let square = Node::path(NodeId(51), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    let mut g = Node::group(NodeId(52), vec![Arc::new(type_node(50)), Arc::new(square)]);
    g.appearance.effects = effects;
    g
}

#[test]
fn type_moved_or_copied_by_a_groups_transform_effect_stays_type() {
    if skip_without_craft_fonts() {
        return;
    }
    for (params, origins, square) in [
        (json!({"moveH": 30}), vec![(50.0, 60.0)], Rect::new(30.0, 0.0, 40.0, 10.0)),
        (json!({"moveH": 200, "copies": 1}), vec![(20.0, 60.0), (220.0, 60.0)], Rect::new(0.0, 0.0, 210.0, 10.0)),
        (json!({"moveV": 50, "copies": 2}), vec![(20.0, 60.0), (20.0, 110.0), (20.0, 160.0)], Rect::new(0.0, 0.0, 10.0, 110.0)),
    ] {
        let g = type_group(vec![fx("distort.transform", params.clone()), fx("stylize.dropShadow", json!({}))]);
        let d = bake_document(&doc_with(g.clone())).unwrap();
        assert_eq!(type_origins(&d), origins, "{params}");
        let baked = d.node(NodeId(52)).unwrap();
        assert_eq!(baked.appearance.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["stylize.dropShadow"]);
        // The square moves with the type, as one path holding its copies.
        let b = baked.children().unwrap().last().and_then(|s| s.geometric_bounds()).unwrap();
        assert!([b.x0 - square.x0, b.y0 - square.y0, b.x1 - square.x1, b.y1 - square.y1].iter().all(|v| v.abs() < 1e-9), "{params}: {b:?}");
        let mut ids = std::collections::HashSet::new();
        baked.walk(&mut |n| assert!(ids.insert(n.id), "{params}: duplicate id {}", n.id));
        // The renderer and Expand Appearance still draw it as outlines.
        let mut text = 0;
        evaluate_container(None, &g).unwrap().walk(&mut |n| text += usize::from(matches!(n.kind, NodeKind::Text(_))));
        assert_eq!(text, 0, "{params}");
    }
    // Stacked effects that only move: every copy of a copy.
    let d = bake_document(&doc_with(type_group(vec![
        fx("distort.transform", json!({"moveH": 100, "copies": 1})),
        fx("distort.transform", json!({"moveV": 100, "copies": 1})),
    ])))
    .unwrap();
    assert_eq!(type_origins(&d), [(20.0, 60.0), (120.0, 60.0), (20.0, 160.0), (120.0, 160.0)]);
}

#[test]
fn hidden_type_in_a_group_that_keeps_its_type_stays_hidden() {
    if skip_without_craft_fonts() {
        return;
    }
    let mut g = type_group(vec![fx("distort.transform", json!({"moveH": 200, "copies": 1}))]);
    let mut hidden = type_node(55);
    hidden.visible = false;
    g.children_mut().unwrap().push(Arc::new(hidden));
    let d = bake_document(&doc_with(g)).unwrap();
    assert_eq!(type_origins(&d), [(20.0, 60.0), (220.0, 60.0)]);
    assert!(!d.node(NodeId(55)).unwrap().visible);
}

#[test]
fn type_outlined_next_to_type_that_stays_type_keeps_its_look() {
    if skip_without_craft_fonts() {
        return;
    }
    let mut g = type_group(vec![fx("distort.transform", json!({"moveH": 200, "copies": 1}))]);
    let mut shadowed = type_node(55);
    shadowed.opacity = 0.5;
    shadowed.appearance.effects.push(fx("stylize.dropShadow", json!({})));
    // Type that clips a group is outlined as the plain outlines it clips by.
    let square = Node::path(NodeId(57), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
    let clip = Node::new(NodeId(58), NodeKind::Group { children: vec![Arc::new(type_node(56)), Arc::new(square)], clip: true });
    g.children_mut().unwrap().extend([Arc::new(shadowed), Arc::new(clip)]);
    let d = bake_document(&doc_with(g)).unwrap();
    assert_eq!(type_origins(&d), [(20.0, 60.0), (220.0, 60.0)]);
    let o = d.node(NodeId(55)).unwrap();
    assert!(matches!(o.kind, NodeKind::Group { .. }) && o.opacity == 0.5, "{o:?}");
    assert_eq!(o.appearance.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["stylize.dropShadow"]);
    // The outlines are copied with the type.
    let b = o.geometric_bounds().unwrap();
    assert!(b.x0 < 30.0 && b.x1 > 220.0, "{b:?}");
    let clipped = d.node(NodeId(58)).and_then(|n| n.children()).and_then(|c| c.first()).unwrap();
    assert!(matches!(clipped.kind, NodeKind::Group { .. }) && clipped.opacity == 1.0, "{clipped:?}");
}

#[test]
fn type_in_a_clipping_group_with_a_fill_of_its_own_is_outlined_plain() {
    if skip_without_craft_fonts() {
        return;
    }
    let mut g = type_group(vec![fx("distort.transform", json!({"moveH": 200, "copies": 1}))]);
    // The clipping path is a group with a fill of its own, holding type with an effect of its own.
    let mut moved = type_node(60);
    moved.appearance.effects.push(fx("distort.transform", json!({"moveV": 300})));
    let mut path = Node::group(NodeId(59), vec![Arc::new(moved)]);
    path.appearance.items.push(AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::BLACK))));
    let square = Node::path(NodeId(57), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
    let clip = Node::new(NodeId(58), NodeKind::Group { children: vec![Arc::new(path), Arc::new(square)], clip: true });
    g.children_mut().unwrap().push(Arc::new(clip));
    let d = bake_document(&doc_with(g)).unwrap();
    assert_eq!(type_origins(&d), [(20.0, 60.0), (220.0, 60.0)]);
    // The type clips by its plain outlines, as the canvas clips by them: its own effect is left out.
    let b = d.node(NodeId(59)).and_then(|n| n.geometric_bounds()).unwrap();
    assert!(b.y0 > 30.0 && b.y1 < 70.0, "{b:?}");
}

#[test]
fn clipping_type_whose_fill_has_a_geometry_effect_clips_by_its_plain_outlines() {
    if skip_without_craft_fonts() {
        return;
    }
    let near = |a: Rect, b: Rect, tol: f64| [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < tol);
    for stroke in [Paint::None, Paint::solid(Color::rgb(1.0, 0.0, 0.0))] {
        // Type that clips a square, with a Transform effect on its own fill.
        let mut t = type_node(60);
        t.appearance = Appearance::basic(Paint::solid(Color::BLACK), stroke.clone(), 1.0).into();
        for item in &mut t.appearance.items {
            if let AppearanceItem::Fill(f) = item {
                f.effects.push(fx("distort.transform", json!({"moveH": 300})));
            }
        }
        let square = Node::path(NodeId(61), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
        let clip = Node::new(NodeId(62), NodeKind::Group { children: vec![Arc::new(t.clone()), Arc::new(square)], clip: true });
        let canvas = crate::clip_outline(&t).map(|(bp, _)| bp.bounding_box()).unwrap();
        assert!(canvas.x1 < 100.0, "the canvas clips by the plain outlines: {canvas:?}");
        let d = bake_document(&doc_with(clip)).unwrap();
        let baked = d.node(NodeId(62)).unwrap();
        // With a stroke, the clip group sits in a group with the stroke over it, outside the clip.
        let group = match baked.kind {
            NodeKind::Group { clip: true, .. } => {
                assert!(stroke.is_none(), "{baked:?}");
                baked
            }
            _ => {
                let ch = baked.children().unwrap();
                assert_eq!(ch.len(), 2, "{baked:?}");
                let strokes = ch[1].appearance.items.iter().all(|i| matches!(i, AppearanceItem::Stroke(_)));
                assert!(matches!(ch[1].kind, NodeKind::Text(_)) && strokes, "the type with only its stroke: {:?}", ch[1]);
                &ch[0]
            }
        };
        let ch = group.children().unwrap();
        assert!(matches!(group.kind, NodeKind::Group { clip: true, .. }) && ch.len() == 3, "{group:?}");
        // The export clips by the outlines the canvas clips by.
        assert!(matches!(ch[0].kind, NodeKind::Path { clipping: true, .. }), "{:?}", ch[0]);
        let b = crate::clip_outline(&ch[0]).map(|(bp, _)| bp.bounding_box()).unwrap();
        assert!(near(b, canvas, 1e-6), "canvas {canvas:?}, export {b:?}");
        // The fills (the characters' and the moved one) paint behind the square, inside the clip.
        let fill = ch[1].geometric_bounds().unwrap();
        assert!((fill.x0 - canvas.x0).abs() < 1e-6 && fill.x1 > canvas.x1 + 250.0, "{fill:?}");
        assert_eq!(ch[2].id, NodeId(61));
        let mut ids = std::collections::HashSet::new();
        baked.walk(&mut |n| assert!(ids.insert(n.id), "duplicate id {}", n.id));
    }
}

#[test]
fn type_with_its_own_transform_effect_copies_stays_type() {
    if skip_without_craft_fonts() {
        return;
    }
    let mut n = type_node(50);
    n.opacity = 0.5;
    n.appearance.effects = vec![fx("distort.transform", json!({"moveV": 50, "copies": 2})), fx("stylize.dropShadow", json!({}))];
    let d = bake_document(&doc_with(n)).unwrap();
    assert_eq!(type_origins(&d), [(20.0, 60.0), (20.0, 110.0), (20.0, 160.0)]);
    // One group keeps the object's transparency and its other effects; the copies are plain.
    let g = d.node(NodeId(50)).unwrap();
    assert!(matches!(g.kind, NodeKind::Group { .. }) && g.opacity == 0.5);
    assert_eq!(g.appearance.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["stylize.dropShadow"]);
    for c in g.children().unwrap() {
        assert!(c.opacity == 1.0 && c.appearance.effects.is_empty() && c.id != g.id, "{c:?}");
    }
    // Hidden type stays hidden.
    let mut n = type_node(50);
    n.visible = false;
    n.appearance.effects = vec![fx("distort.transform", json!({"moveV": 50, "copies": 1}))];
    let d = bake_document(&doc_with(n)).unwrap();
    assert_eq!(type_origins(&d).len(), 2);
    assert!(!d.node(NodeId(50)).unwrap().visible);
}

#[test]
fn type_under_other_geometry_effects_is_still_outlined() {
    for effects in [
        vec![fx("distort.transform", json!({"moveH": 30, "random": true}))],
        vec![fx("distort.transform", json!({"moveH": 30, "scaleH": 50}))],
        vec![fx("distort.transform", json!({"moveH": 30, "rotate": 10, "copies": 1}))],
        vec![fx("distort.transform", json!({"moveH": 30, "reflectX": true}))],
        vec![fx("distort.transform", json!({"moveH": 30})), fx("distort.roughen", json!({"size": 4}))],
        // Copies that would reach past the effect's limit, which stops them early.
        vec![fx("distort.transform", json!({"moveH": 1.0e7, "copies": 1000}))],
        // Copies that overlap, or land on each other.
        vec![fx("distort.transform", json!({"moveH": 5, "copies": 1}))],
        vec![fx("distort.transform", json!({"moveH": 100, "copies": 1})), fx("distort.transform", json!({"moveH": -100, "copies": 1}))],
    ] {
        let mut own = type_node(50);
        own.appearance.effects = effects.clone();
        for n in [type_group(effects.clone()), own] {
            let d = bake_document(&doc_with(n)).unwrap();
            assert!(type_origins(&d).is_empty(), "{effects:?}");
        }
    }
}

#[test]
fn type_whose_copies_paint_over_each_other_is_still_outlined() {
    if skip_without_craft_fonts() {
        return;
    }
    let moves = vec![fx("distort.transform", json!({"moveV": 40, "copies": 1}))];
    assert_eq!(type_origins(&bake_document(&doc_with(type_group(moves.clone()))).unwrap()).len(), 2);
    // A 20 pt stroke on the characters reaches the copy below.
    let mut stroked = type_node(50);
    if let NodeKind::Text(t) = &mut stroked.kind {
        t.runs[0].style.stroke = Paint::solid(Color::BLACK);
        t.runs[0].style.stroke_width = 20.0;
    }
    let mut g = type_group(moves);
    g.children_mut().unwrap()[0] = Arc::new(stroked);
    assert!(type_origins(&bake_document(&doc_with(g)).unwrap()).is_empty());
}

#[test]
fn type_with_a_look_of_its_own_in_a_moved_group_is_still_outlined() {
    if skip_without_craft_fonts() {
        return;
    }
    let moves = vec![fx("distort.transform", json!({"moveV": 40, "copies": 1}))];
    let mut translucent = type_node(50);
    translucent.opacity = 0.5;
    let mut underlined = type_node(50);
    if let NodeKind::Text(t) = &mut underlined.kind {
        t.runs[0].style.underline = true;
    }
    let mut shadowed = type_node(50);
    shadowed.appearance.effects.push(fx("stylize.dropShadow", json!({})));
    let mut gradient = type_node(50);
    if let NodeKind::Text(t) = &mut gradient.kind {
        t.runs[0].style.fill = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default())));
    }
    let mut reshaped = type_node(50);
    let mut fill = FillLayer::new(Paint::solid(Color::BLACK));
    fill.effects.push(fx("path.offsetPath", json!({"offset": 2})));
    reshaped.appearance.items.push(AppearanceItem::Fill(fill));
    // A stroke of the type object's own (beyond its characters').
    let mut stroked = type_node(50);
    stroked.appearance.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 1.0)));
    // Character strokes scale with type scaled by its transform, where the outlines keep theirs.
    let mut scaled = type_node(50);
    if let NodeKind::Text(t) = &mut scaled.kind {
        t.runs[0].style.stroke = Paint::solid(Color::BLACK);
        t.runs[0].style.stroke_width = 1.0;
        t.xf *= Affine::scale(2.0);
    }
    let mut turned = scaled.clone();
    if let NodeKind::Text(t) = &mut turned.kind {
        t.xf = Affine::translate((20.0, 60.0)) * Affine::rotate(0.5);
    }
    let far = vec![fx("distort.transform", json!({"moveH": 300, "copies": 1}))];
    for (t, kept) in [(turned, 2), (scaled, 0)] {
        let mut g = type_group(far.clone());
        g.children_mut().unwrap()[0] = Arc::new(t);
        assert_eq!(type_origins(&bake_document(&doc_with(g)).unwrap()).len(), kept);
    }
    for t in [translucent, underlined, shadowed, gradient, reshaped, stroked] {
        let mut g = type_group(moves.clone());
        g.children_mut().unwrap()[0] = Arc::new(t);
        assert!(type_origins(&bake_document(&doc_with(g)).unwrap()).is_empty());
    }
}

#[test]
fn type_in_a_moved_group_with_a_fill_of_its_own_is_still_outlined() {
    let moves = vec![fx("distort.transform", json!({"moveV": 100, "copies": 1}))];
    let fill = AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::BLACK)));
    let mut g = type_group(moves.clone());
    g.appearance.items.push(fill.clone());
    assert!(type_origins(&bake_document(&doc_with(g)).unwrap()).is_empty());
    // A group with a fill of its own inside the moved group.
    let mut inner = type_group(vec![]);
    inner.id = NodeId(53);
    inner.appearance.items.push(fill);
    let mut outer = Node::group(NodeId(54), vec![Arc::new(inner)]);
    outer.appearance.effects = moves;
    assert!(type_origins(&bake_document(&doc_with(outer)).unwrap()).is_empty());
}

#[test]
fn type_in_a_moved_group_inside_a_moved_group_is_still_outlined() {
    // The inner copy lands back on the original.
    let mut inner = type_group(vec![fx("distort.transform", json!({"moveH": -200, "copies": 1}))]);
    inner.id = NodeId(53);
    let mut outer = Node::group(NodeId(54), vec![Arc::new(inner)]);
    outer.appearance.effects.push(fx("distort.transform", json!({"moveH": 200, "copies": 1})));
    assert!(type_origins(&bake_document(&doc_with(outer)).unwrap()).is_empty());
}

#[test]
fn type_whose_copies_hold_too_much_is_still_outlined() {
    if skip_without_craft_fonts() {
        return;
    }
    let spaced = |spaces: usize| {
        let mut n = type_node(50);
        if let NodeKind::Text(t) = &mut n.kind {
            t.runs[0].text = format!("a{}", " ".repeat(spaces));
        }
        Arc::new(n)
    };
    // 1001 copies of about 2,000 bytes stay type; of about 21,000 bytes each, past 20,000,000
    // bytes together, they are outlined.
    let copies = vec![fx("distort.transform", json!({"moveH": 100, "copies": 1000}))];
    for (spaces, kept) in [(1000, 1001), (20_000, 0)] {
        let mut g = type_group(copies.clone());
        g.children_mut().unwrap()[0] = spaced(spaces);
        assert_eq!(type_origins(&bake_document(&doc_with(g)).unwrap()).len(), kept, "{spaces} spaces");
    }
    // 1001 × 21 copies, past 10,000.
    let mut g = type_group(vec![
        fx("distort.transform", json!({"moveH": 100, "copies": 1000})),
        fx("distort.transform", json!({"moveV": 100, "copies": 20})),
    ]);
    g.children_mut().unwrap()[0] = spaced(0);
    assert!(type_origins(&bake_document(&doc_with(g)).unwrap()).is_empty());
}

#[test]
fn copies_are_apart_when_their_boxes_only_touch() {
    let b = Rect::new(0.0, 0.0, 10.0, 5.0);
    assert!(apart(&[Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(0.0, 5.0), Vec2::new(-10.0, -5.0)], b));
    assert!(!apart(&[Vec2::ZERO, Vec2::new(9.0, 4.0)], b));
    assert!(!apart(&[Vec2::new(100.0, 0.0), Vec2::new(30.0, 0.0), Vec2::new(100.0, 0.0)], b));
    assert!(!apart(&[Vec2::new(-1.0, -1.0), Vec2::new(8.5, 3.5)], b));
    // Offsets too far for the cells' range count as overlapping.
    assert!(!apart(&[Vec2::ZERO, Vec2::new(1.0e9, 0.0)], Rect::new(0.0, 0.0, 1.0e-9, 1.0)));
}

#[test]
fn type_in_a_pathfinder_group_inside_a_moved_group_is_still_outlined() {
    if skip_without_craft_fonts() {
        return;
    }
    let mut inner = type_group(vec![fx("pathfinder.add", json!({}))]);
    inner.id = NodeId(53);
    let mut outer = Node::group(NodeId(54), vec![Arc::new(inner)]);
    outer.appearance.effects.push(fx("distort.transform", json!({"moveV": 100, "copies": 1})));
    let d = bake_document(&doc_with(outer)).unwrap();
    assert!(type_origins(&d).is_empty());
    // The glyphs take part in Add: the result reaches past the 10 pt square.
    let b = d.node(NodeId(54)).and_then(|n| n.geometric_bounds()).unwrap();
    assert!(b.width() > 30.0, "{b:?}");
}

#[test]
fn type_is_copied_only_when_the_effect_makes_every_copy() {
    // A 10 pt line with `n` anchors.
    let line = |n: usize| {
        let pts: Vec<Point> = (0..n).map(|i| Point::new(i as f64 * 10.0 / n as f64, 0.0)).collect();
        PathData::new(vec![vectorcraft_geom::SubPath::polyline(&pts, false)])
    };
    let right = json!({"moveH": 10, "copies": 1000});
    let down = json!({"moveV": 10, "copies": 1});
    // 1000 copies of a path stay within 2,000,000 anchors up to 1998 anchors.
    assert_eq!(transform_offsets(std::slice::from_ref(&right), &line(1998)).map(|o| o.len()), Some(1001));
    assert!(transform_offsets(std::slice::from_ref(&right), &line(1999)).is_none());
    // A second effect copies every copy.
    assert_eq!(transform_offsets(&[right.clone(), down.clone()], &line(999)).map(|o| o.len()), Some(2002));
    assert!(transform_offsets(&[right, down], &line(1000)).is_none());
    // Copies stop before one reaches 1,000,000,000 pt from the origin.
    let square = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(transform_offsets(&[json!({"moveH": 1.0e7, "copies": 99})], &square).map(|o| o.len()), Some(100));
    assert!(transform_offsets(&[json!({"moveH": 1.0e7, "copies": 100})], &square).is_none());
    // An effect that also scales.
    assert!(transform_offsets(&[json!({"moveH": 10, "scaleH": 50})], &square).is_none());
}
/// A 10 pt square painted with a red to blue gradient and a Transform effect with 2 copies.
fn gradient_copies(id: u64) -> Node {
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let paint = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let mut n = Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(paint, Paint::None, 0.0));
    n.appearance.effects.push(fx("distort.transform", json!({"moveH": 30, "copies": 2})));
    n
}

/// The paths under `n` that paint a gradient fitted to their own 10 pt box, left to right.
fn gradient_pieces(n: &Node) -> Vec<f64> {
    let mut out = vec![];
    let mut walk = vec![n];
    while let Some(n) = walk.pop() {
        match n.children() {
            Some(ch) if !matches!(n.kind, NodeKind::Compound { .. }) => walk.extend(ch.iter().map(|c| &**c)),
            _ => {
                assert!(matches!(n.appearance.fill_paint(), Paint::Gradient(_)), "{n:?}");
                out.extend(n.geometric_bounds().map(|b| b.width()));
            }
        }
    }
    out
}

#[test]
fn exporting_a_path_with_a_transform_gives_each_copy_its_own_gradient() {
    let d = bake_document(&doc_with(gradient_copies(50))).unwrap();
    // Three squares of their own, not one path spanning all three.
    assert_eq!(gradient_pieces(d.node(NodeId(50)).unwrap()), vec![10.0; 3]);
}
