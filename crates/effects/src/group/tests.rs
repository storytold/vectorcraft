use super::*;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{CharStyle, Effect, NodeId, TextObject};
use vectorcraft_geom::{Point, Rect, shapes};

/// Type tests need glyph outlines, which only the optional craft-fonts build input provides.
fn skip_without_craft_fonts() -> bool {
    let skip = vectorcraft_text::CRAFT_FONTS.is_empty();
    if skip {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
    }
    skip
}

fn sq(id: u64, x: f64, color: Color) -> Arc<Node> {
    Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::basic(Paint::solid(color), Paint::None, 0.0)))
}

fn group_with(effect: &str) -> Node {
    let mut g = Node::group(NodeId(10), vec![sq(1, 0.0, Color::rgb(1.0, 0.0, 0.0)), sq(2, 5.0, Color::rgb(0.0, 0.0, 1.0))]);
    g.appearance.effects.push(Effect { id: effect.into(), params: serde_json::json!({}), visible: true });
    g
}

fn area(n: &Node) -> f64 {
    po::area(n.path_data().unwrap(), FillRule::NonZero).abs()
}

fn overlap_cmyk(g: &Node) -> [f32; 4] {
    let faces = pathfinder_children(g, None).unwrap();
    assert_eq!(faces.len(), 3);
    let overlap = faces
        .iter()
        .find(|n| {
            let b = n.geometric_bounds().unwrap();
            b.x0 >= 4.9 && b.x1 <= 10.1 && b.y0 <= 0.1 && b.y1 >= 9.9
        })
        .expect("overlap face");
    assert!(overlap.appearance.stroke_paint().is_none());
    overlap.appearance.fill_paint().color().unwrap().to_cmyk()
}

fn two_overlapping_cmyk_squares() -> Node {
    let a = Appearance::basic(Paint::solid(Color::cmyk(0.2, 0.66, 0.4, 0.0)), Paint::solid(Color::BLACK), 2.0);
    let b = Appearance::basic(Paint::solid(Color::cmyk(0.4, 0.2, 0.3, 0.1)), Paint::solid(Color::BLACK), 2.0);
    Node::group(
        NodeId(10),
        vec![
            Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), a)),
            Arc::new(Node::path(NodeId(2), shapes::rectangle(Rect::new(5.0, 0.0, 15.0, 10.0)), b)),
        ],
    )
}

/// Effect › Pathfinder › Trap keeps the members and adds an overprinting strip of the lighter
/// colour inside the darker one along the edge where they meet.
#[test]
fn trap_effect_adds_an_overprinting_strip_above_the_members() {
    let yellow = Color::cmyk(0.0, 0.0, 1.0, 0.0);
    let blue = Color::cmyk(1.0, 0.6, 0.0, 0.0);
    let mut g = Node::group(NodeId(10), vec![sq(1, 0.0, yellow), sq(2, 10.0, blue)]);
    let params = serde_json::json!({"thickness": 1.0, "tintReduction": 100});
    g.appearance.effects.push(Effect { id: crate::trap::TRAP.into(), params, visible: true });
    let out = pathfinder_children(&g, None).unwrap();
    assert_eq!(out.len(), 3, "both squares and one trap");
    assert_eq!((out[0].id, out[1].id), (NodeId(1), NodeId(2)));
    let b = out[2].geometric_bounds().unwrap();
    assert!((b.x0 - 10.0).abs() < 0.02 && (b.x1 - 11.0).abs() < 0.02 && b.y0.abs() < 0.02 && (b.y1 - 10.0).abs() < 0.02, "{b:?}");
    let fill = out[2].appearance.fill().unwrap();
    assert!(fill.overprint);
    assert_eq!(fill.paint.color(), Some(yellow));
}

#[test]
fn hard_mix_takes_the_highest_of_each_cmyk_channel_and_drops_strokes() {
    let mut g = two_overlapping_cmyk_squares();
    g.appearance.effects.push(Effect { id: "pathfinder.hardMix".into(), params: serde_json::json!({}), visible: true });
    let [c, m, y, k] = overlap_cmyk(&g);
    assert!((c - 0.4).abs() < 0.02 && (m - 0.66).abs() < 0.02 && (y - 0.4).abs() < 0.02 && (k - 0.1).abs() < 0.02, "{c} {m} {y} {k}");
}

#[test]
fn soft_mix_blends_cmyk_bottom_to_top_and_drops_strokes() {
    let mut g = two_overlapping_cmyk_squares();
    g.appearance.effects.push(Effect { id: "pathfinder.softMix".into(), params: serde_json::json!({"mixingRate": 50.0}), visible: true });
    let [c, m, y, k] = overlap_cmyk(&g);
    assert!((c - 0.3).abs() < 0.02 && (m - 0.43).abs() < 0.02 && (y - 0.35).abs() < 0.02 && (k - 0.05).abs() < 0.02, "{c} {m} {y} {k}");
}

#[test]
fn shape_modes_follow_the_pathfinder_panel() {
    let add = pathfinder_children(&group_with("pathfinder.add"), None).unwrap();
    assert_eq!(add.len(), 1);
    assert!((area(&add[0]) - 150.0).abs() < 1e-3);
    // Add keeps the front object's paint, Subtract the back one's.
    assert_eq!(add[0].id, NodeId(2));
    let sub = pathfinder_children(&group_with("pathfinder.subtract"), None).unwrap();
    assert_eq!(sub[0].id, NodeId(1));
    assert!((area(&sub[0]) - 50.0).abs() < 1e-3);
    let int = pathfinder_children(&group_with("pathfinder.intersect"), None).unwrap();
    assert!((area(&int[0]) - 50.0).abs() < 1e-3);
    let div = pathfinder_children(&group_with("pathfinder.divide"), None).unwrap();
    assert_eq!(div.len(), 3);
}

/// Returns an "H" in blue type at 100 pt on a baseline at y = 80, with its stems on either
/// side of x = 100.
fn type_h() -> Node {
    let style = CharStyle { size: 100.0, fill: Paint::solid(Color::rgb(0.0, 0.0, 1.0)), ..CharStyle::default() };
    Node::new(NodeId(2), NodeKind::Text(Box::new(TextObject::point(Point::new(70.0, 80.0), "H", style))))
}

/// Returns a group with `effect` that holds a red 100 x 100 square and `front` in front of it.
fn square_and(front: Option<Node>, effect: &str) -> Node {
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let square = Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), red);
    let mut g = Node::group(NodeId(10), [Some(square), front].into_iter().flatten().map(Arc::new).collect());
    g.appearance.effects.push(Effect { id: effect.into(), params: serde_json::json!({}), visible: true });
    g
}

/// Returns the Pathfinder results of `g`, with type outlined as the renderer outlines it.
fn results(g: &Node) -> Vec<Arc<Node>> {
    pathfinder_children(g, crate::text_outliner()).unwrap()
}

#[test]
fn type_changes_the_result_of_every_pathfinder_effect() {
    if skip_without_craft_fonts() {
        return;
    }
    let glyphs = crate::outline_text(&type_h()).unwrap();
    let b = glyphs.geometric_bounds().unwrap();
    assert!(b.x0 < 100.0 && b.x1 > 100.0 && b.y0 > 0.0 && b.y1 < 100.0, "{b:?}");
    // `measure` gives the number of results and their total area.
    let measure = |front: Option<Node>, effect: &str| {
        let r = results(&square_and(front, effect));
        (r.len(), r.iter().map(|n| area(n)).sum::<f64>())
    };
    let unchanged: Vec<_> = PATHFINDER_EFFECTS
        .iter()
        .map(|(id, label, _)| (label, measure(Some(type_h()), id), measure(None, id)))
        .filter(|(_, with, without)| with.0 == without.0 && (with.1 - without.1).abs() <= 1.0)
        .collect();
    assert!(unchanged.is_empty(), "(label, with the type, without): {unchanged:?}");
    // The shape modes take the glyph's whole area: the part inside the square and the part outside.
    let glyph_area: f64 = glyphs.children().unwrap().iter().map(|n| area(n)).sum();
    let with_type = |id: &str| measure(Some(type_h()), id).1;
    assert!((with_type("pathfinder.add") - with_type("pathfinder.subtract") - glyph_area).abs() < 0.5, "{glyph_area}");
    assert!((with_type("pathfinder.intersect") + with_type("pathfinder.minusBack") - glyph_area).abs() < 0.5, "{glyph_area}");
    // Outline art nested one group deeper takes part as well.
    let nested: OutlineHook = &|n: &Node| crate::outline_text(n).map(|g| Node::group(n.id, vec![Arc::new(g)]));
    let sub = pathfinder_children(&square_and(Some(type_h()), "pathfinder.subtract"), Some(nested)).unwrap();
    assert!((sub.iter().map(|n| area(n)).sum::<f64>() - with_type("pathfinder.subtract")).abs() < 1e-6);
}

#[test]
fn pieces_of_type_keep_its_paint_opacity_and_blend_mode() {
    if skip_without_craft_fonts() {
        return;
    }
    let mut t = type_h();
    (t.opacity, t.blend) = (0.5, BlendMode::Multiply);
    let look = |p: &Arc<Node>| (p.appearance.fill_paint(), p.opacity, p.blend);
    let square = (Paint::solid(Color::rgb(1.0, 0.0, 0.0)), 1.0, BlendMode::Normal);
    let glyph = (Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 0.5, BlendMode::Multiply);
    let looks: Vec<_> = results(&square_and(Some(t.clone()), "pathfinder.divide")).iter().map(look).collect();
    assert!(looks.contains(&square) && looks.contains(&glyph) && looks.iter().all(|l| *l == square || *l == glyph), "{looks:?}");
    // In the shape modes a group with the type in front is one shape, styled like the type.
    let green = Appearance::basic(Paint::solid(Color::rgb(0.0, 1.0, 0.0)), Paint::None, 0.0);
    let behind = Node::path(NodeId(3), shapes::rectangle(Rect::new(50.0, 50.0, 150.0, 150.0)), green);
    let both = Node::group(NodeId(4), vec![Arc::new(behind), Arc::new(t)]);
    let added: Vec<_> = results(&square_and(Some(both), "pathfinder.add")).iter().map(look).collect();
    assert_eq!(added, [glyph]);
}

#[test]
fn a_random_transform_on_a_group_moves_its_members_together() {
    let mut g = group_with("distort.transform");
    g.appearance.effects[0].params = serde_json::json!({"moveH": 100, "moveV": 100, "random": true});
    let art = evaluate_container(None, &g).unwrap();
    let corner = |i: usize| art.children().and_then(|c| c.get(i)).and_then(|n| n.geometric_bounds()).map(|b| (b.x0, b.y0)).unwrap();
    let (a, b) = (corner(0), corner(1));
    assert!(a.0 > 0.0 && a.0 < 100.0, "moved by a share of the value: {a:?}");
    assert!((b.0 - a.0 - 5.0).abs() < 1e-9 && (b.1 - a.1).abs() < 1e-9, "{a:?} {b:?}");
}

/// What [`evaluate_container`] makes of black 40 pt type "IIII" (id 3), changed by `style`,
/// in a group with a square and Offset Path 0, which leaves every path as it is.
fn type_in_group(style: impl FnOnce(&mut Node)) -> Node {
    let ts = vectorcraft_doc::CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(5.0, 60.0), "IIII", ts);
    let mut t = Node::new(NodeId(3), NodeKind::Text(Box::new(text)));
    style(&mut t);
    let mut g = Node::group(NodeId(10), vec![sq(1, 0.0, Color::BLACK), Arc::new(t)]);
    g.appearance.effects.push(Effect { id: "path.offsetPath".into(), params: serde_json::json!({"offset": 0}), visible: true });
    let art = evaluate_container(None, &g).unwrap();
    (*art.children().unwrap()[1]).clone()
}

#[test]
fn type_in_a_group_keeps_its_look_when_the_groups_geometry_effects_outline_it() {
    if skip_without_craft_fonts() {
        return;
    }
    use vectorcraft_color::BlendMode;
    let art = type_in_group(|t| (t.opacity, t.blend) = (0.5, BlendMode::Multiply));
    assert!(art.id == NodeId(3) && !matches!(art.kind, NodeKind::Text(_)), "{art:?}");
    assert_eq!((art.opacity, art.blend), (0.5, BlendMode::Multiply));
    // Its opacity mask, its own shadow, its knockout shape option and its visibility stay.
    let mask_art = Node::path(NodeId(4), shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default());
    let mask = vectorcraft_doc::OpacityMask::new(mask_art, true);
    assert_eq!(type_in_group(|t| t.extras_mut().mask = Some(Box::new(mask.clone()))).extras().mask.as_deref(), Some(&mask));
    let shadow = Effect { id: "stylize.dropShadow".into(), params: serde_json::json!({}), visible: true };
    assert_eq!(type_in_group(|t| t.appearance.effects.push(shadow.clone())).appearance.effects, vec![shadow]);
    assert!(type_in_group(|t| t.knockout_shape = true).knockout_shape);
    assert!(!type_in_group(|t| t.visible = false).visible);
    // Its own Transform moves its outlines 30 pt before the group's effect reshapes them.
    let x0 = |n: Node| n.geometric_bounds().unwrap().x0;
    let transform = Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 30}), visible: true };
    let moved = x0(type_in_group(|t| t.appearance.effects.push(transform)));
    assert!((moved - x0(type_in_group(|_| {})) - 30.0).abs() < 1e-6, "{moved}");
}

#[test]
fn hidden_effect_or_plain_group_is_untouched() {
    let mut g = group_with("pathfinder.add");
    g.appearance.effects[0].visible = false;
    assert!(pathfinder_children(&g, None).is_none());
    assert!(!has_pathfinder(&sq(1, 0.0, Color::BLACK)));
}

#[test]
fn a_transform_on_a_group_moves_the_gradients_of_its_type_and_not_those_placed_on_its_paths() {
    if skip_without_craft_fonts() {
        return;
    }
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    // The type's characters fit the gradient to its layout; the square has it placed across itself.
    let style = vectorcraft_doc::CharStyle { size: 40.0, fill: Paint::Gradient(Box::new(ramp.clone())), ..Default::default() };
    let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(5.0, 60.0), "IIII", style);
    let t = Arc::new(Node::new(NodeId(3), NodeKind::Text(Box::new(text))));
    let geom = GradientGeom { start: (0.0, 5.0).into(), end: (10.0, 5.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let square = Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(placed, Paint::None, 0.0)));
    let start = |n: &Node| match n.appearance.fill_paint() {
        Paint::Gradient(g) => g.geom.unwrap().start.x,
        p => panic!("a gradient: {p:?}"),
    };
    let before = start(&crate::outline_text(&t).unwrap().children().unwrap()[0]);
    let mut g = Node::group(NodeId(10), vec![square, t]);
    g.appearance.effects.push(Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 30}), visible: true });
    let art = evaluate_container(None, &g).unwrap();
    let members = art.children().unwrap();
    let after = start(&members[1].children().unwrap()[0]);
    assert!((after - before - 30.0).abs() < 1e-6, "the type's gradient starts at {before}, then {after}");
    assert_eq!(start(&members[0]), 0.0, "the square's gradient stays where it was placed");
}

#[test]
fn a_transform_on_a_group_paints_each_copy_with_its_own_gradient() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (0.0, 5.0).into(), end: (10.0, 5.0).into(), aspect: 1.0, focal: None };
    let paints = [Paint::Gradient(Box::new(ramp.clone())), Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }))];
    for (k, paint) in paints.into_iter().enumerate() {
        let square = Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(paint, Paint::None, 0.0)));
        let mut g = Node::group(NodeId(10), vec![square]);
        let params = serde_json::json!({"moveH": 30, "copies": 2});
        g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
        let art = evaluate_container(None, &g).unwrap();
        let copies = art.children().unwrap()[0].children().expect("the object and its copies are objects of their own");
        assert_eq!(copies.len(), 3);
        for (i, c) in copies.iter().enumerate() {
            // Each copy's own box: an unplaced gradient fits it, a placed one moved with it.
            assert!((c.geometric_bounds().unwrap().x0 - 30.0 * i as f64).abs() < 1e-6);
            match c.appearance.fill_paint() {
                Paint::Gradient(g) if k == 0 => assert!(g.geom.is_none(), "copy {i} fits its own box"),
                Paint::Gradient(g) => assert!((g.geom.unwrap().start.x - 30.0 * i as f64).abs() < 1e-6, "copy {i}"),
                p => panic!("a gradient: {p:?}"),
            }
        }
    }
}

/// A group with `effect` holding `n` vertical then `n` horizontal bars, 2 wide on a 4-unit
/// pitch, each crossing every bar of the other direction: n² crossings.
fn crossing_bars(n: usize, effect: &str) -> Node {
    let len = 4.0 * n as f64;
    let paint = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let mut children = vec![];
    for i in 0..n {
        let at = 4.0 * i as f64;
        children.push(Arc::new(Node::path(NodeId(100 + i as u64), shapes::rectangle(Rect::new(at, 0.0, at + 2.0, len)), paint.clone())));
    }
    for i in 0..n {
        let at = 4.0 * i as f64;
        let id = NodeId(100 + (n + i) as u64);
        children.push(Arc::new(Node::path(id, shapes::rectangle(Rect::new(0.0, at, len, at + 2.0)), paint.clone())));
    }
    let mut g = Node::group(NodeId(10), children);
    g.appearance.effects.push(Effect { id: effect.into(), params: serde_json::json!({}), visible: true });
    g
}

#[test]
fn pathfinder_past_its_budget_draws_the_source_art() {
    // n² crossings past the cap: no evaluation (#3591), the source art as it is.
    for n in [130, 800] {
        let g = crossing_bars(n, "pathfinder.divide");
        let out = pathfinder_children(&g, None).unwrap();
        assert_eq!(out.len(), 2 * n, "{n} x {n} bars");
        let ids: Vec<NodeId> = out.iter().map(|c| c.id).collect();
        let src: Vec<NodeId> = g.children().unwrap().iter().map(|c| c.id).collect();
        assert_eq!(ids, src, "{n} x {n} bars");
        assert!(out.iter().zip(g.children().unwrap()).all(|(a, b)| a.path_data() == b.path_data()));
    }
}

#[test]
fn pathfinder_within_its_budget_still_divides() {
    // Each bar is cut into one piece per crossing plus the n pieces between and past them, so
    // n x n bars divide into 3·n² pieces, and 128 x 128 bars, which overlap in exactly
    // `PATHFINDER_MAX_OVERLAPS` pairs, must still divide.
    for n in [8, 128] {
        let out = pathfinder_children(&crossing_bars(n, "pathfinder.divide"), None).unwrap();
        assert_eq!(out.len(), 3 * n * n, "{n} x {n} bars");
    }
}

#[test]
fn pathfinder_on_many_stacked_bars_still_runs() {
    // 3000 bars stacked in y all overlap in x, but each overlaps only the next one in y: 2999
    // overlapping pairs are within the budget, so Add still merges them into one path.
    let paint = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let children = (0..3000)
        .map(|i| {
            let at = 1.5 * i as f64;
            Arc::new(Node::path(NodeId(100 + i as u64), shapes::rectangle(Rect::new(0.0, at, 100.0, at + 2.0)), paint.clone()))
        })
        .collect();
    let mut g = Node::group(NodeId(10), children);
    g.appearance.effects.push(Effect { id: "pathfinder.add".into(), params: serde_json::json!({}), visible: true });
    let out = pathfinder_children(&g, None).unwrap();
    assert_eq!(out.len(), 1);
}
