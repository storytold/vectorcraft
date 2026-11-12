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
    let faces = pathfinder_children(None, g, None).unwrap();
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
    let out = pathfinder_children(None, &g, None).unwrap();
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
    let add = pathfinder_children(None, &group_with("pathfinder.add"), None).unwrap();
    assert_eq!(add.len(), 1);
    assert!((area(&add[0]) - 150.0).abs() < 1e-3);
    // Add keeps the front object's paint, Subtract the back one's.
    assert_eq!(add[0].id, NodeId(2));
    let sub = pathfinder_children(None, &group_with("pathfinder.subtract"), None).unwrap();
    assert_eq!(sub[0].id, NodeId(1));
    assert!((area(&sub[0]) - 50.0).abs() < 1e-3);
    let int = pathfinder_children(None, &group_with("pathfinder.intersect"), None).unwrap();
    assert!((area(&int[0]) - 50.0).abs() < 1e-3);
    let div = pathfinder_children(None, &group_with("pathfinder.divide"), None).unwrap();
    assert_eq!(div.len(), 3);
}

/// Divide and Outline draw no piece of a member without fill or stroke, the same as when such
/// pieces are left out.
#[test]
fn divide_and_outline_draw_nothing_for_unpainted_members() {
    let unpainted =
        Arc::new(Node::path(NodeId(2), shapes::rectangle(Rect::new(5.0, 0.0, 15.0, 10.0)), Appearance::basic(Paint::None, Paint::None, 0.0)));
    for effect in ["pathfinder.divide", "pathfinder.outline"] {
        let mut g = Node::group(NodeId(10), vec![sq(1, 0.0, Color::rgb(1.0, 0.0, 0.0)), unpainted.clone()]);
        g.appearance.effects.push(Effect { id: effect.into(), params: serde_json::json!({}), visible: true });
        let out = pathfinder_children(None, &g, None).unwrap();
        assert!(out.iter().any(|n| n.id == NodeId(1)) && out.iter().any(|n| n.id == NodeId(2)), "{effect}: {out:#?}");
        for n in out.iter().filter(|n| n.id == NodeId(2)) {
            assert!(n.appearance.items.iter().all(|i| i.paint().is_none()), "{effect}: {n:#?}");
        }
    }
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
    pathfinder_children(None, g, crate::text_outliner()).unwrap()
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
    let sub = pathfinder_children(None, &square_and(Some(type_h()), "pathfinder.subtract"), Some(nested)).unwrap();
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
    assert!(pathfinder_children(None, &g, None).is_none());
    assert!(!has_pathfinder(&sq(1, 0.0, Color::BLACK)));
}

#[test]
fn a_transform_on_a_group_moves_the_gradients_of_its_type_and_those_placed_on_its_paths() {
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
    assert!((start(&members[0]) - 30.0).abs() < 1e-6, "the square's gradient moves with it");
}

#[test]
fn a_groups_transform_moves_the_gradient_of_its_own_fill() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (0.0, 5.0).into(), end: (15.0, 5.0).into(), aspect: 1.0, focal: None };
    let mut g = Node::group(NodeId(10), vec![sq(1, 0.0, Color::BLACK), sq(2, 5.0, Color::BLACK)]);
    g.appearance
        .items
        .push(AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp })))));
    g.appearance.effects.push(Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 30}), visible: true });
    let art = evaluate_container(None, &g).unwrap();
    let mut starts = vec![];
    art.walk(&mut |c: &Node| {
        if let Paint::Gradient(p) = c.appearance.fill_paint() {
            starts.push(p.geom.unwrap().start.x);
        }
    });
    assert!(!starts.is_empty() && starts.iter().all(|s| (s - 30.0).abs() < 1e-6), "{starts:?}");
}

/// The art Pathfinder and compound shapes take (`shape_art`) paints as the canvas does: a path's
/// gradient and a group's own fill move with a Transform effect.
#[test]
fn shape_art_moves_gradients_with_a_transform_effect() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (0.0, 5.0).into(), end: (10.0, 5.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let moved = || Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 40}), visible: true };
    let starts = |art: &Node| {
        let mut out = vec![];
        art.walk(&mut |c: &Node| {
            for item in &c.appearance.items {
                if let Paint::Gradient(p) = item.paint() {
                    out.push(p.geom.unwrap().start.x);
                }
            }
        });
        out
    };
    let mut path = Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(placed.clone(), Paint::None, 0.0));
    path.appearance.effects.push(moved());
    let got = starts(&shape_art(None, &path, None).unwrap());
    assert!(got.len() == 1 && (got[0] - 40.0).abs() < 1e-6, "the path's gradient: {got:?}");
    let mut g = Node::group(NodeId(10), vec![sq(2, 0.0, Color::BLACK)]);
    g.appearance.items.push(AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(placed)));
    g.appearance.effects.push(moved());
    let got = starts(&shape_art(None, &g, None).unwrap());
    assert!(!got.is_empty() && got.iter().all(|s| (s - 40.0).abs() < 1e-6), "the group's own fill: {got:?}");
}

/// A symbol instance's Transform effect moves the gradient placed on its art's own fill in the
/// art Pathfinder takes, as it moves its members.
#[test]
fn shape_art_moves_the_gradient_of_a_symbol_instances_art_with_its_transform() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (0.0, 5.0).into(), end: (10.0, 5.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let mut art = Node::group(NodeId(20), vec![sq(21, 0.0, Color::BLACK)]);
    art.appearance.items.push(AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(placed)));
    let hook = move |_: &Node| Some(art.clone());
    let mut instance = Node::new(NodeId(1), NodeKind::SymbolInstance { symbol: "s".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    instance.appearance.effects.push(Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 30}), visible: true });
    let out = shape_art(None, &instance, Some(&hook)).unwrap();
    let Paint::Gradient(g) = out.appearance.fill_paint() else { panic!("a gradient: {:?}", out.appearance) };
    assert!((g.geom.unwrap().start.x - 30.0).abs() < 1e-6, "{:?}", g.geom);
    assert!((out.geometric_bounds().unwrap().x0 - 30.0).abs() < 1e-6, "the art moves");
}

/// Under a Warp effect, the gradient placed on a symbol instance's art's own fill follows the art's
/// bounds, and the gradient placed on a compound shape follows its area's bounds, each from before
/// the warp to after it, in the art Pathfinder takes.
#[test]
fn shape_art_moves_the_gradients_of_a_symbol_instances_art_and_a_compound_shape_with_a_warp() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (2.0, 3.0).into(), end: (9.0, 3.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let follow = |p: Point, from: Rect, to: Rect| {
        Point::new(to.x0 + (p.x - from.x0) / from.width() * to.width(), to.y0 + (p.y - from.y0) / from.height() * to.height())
    };
    let check = |out: &Node, from: Rect, what: &str| {
        let to = out.geometric_bounds().unwrap();
        assert_ne!(from, to, "{what}: the warp changes the bounds");
        let Paint::Gradient(g) = out.appearance.fill_paint() else { panic!("{what}: a gradient: {:?}", out.appearance) };
        let got = g.geom.unwrap();
        let (s, e) = (follow(geom.start, from, to), follow(geom.end, from, to));
        assert!(got.start.distance(s) < 1e-6 && got.end.distance(e) < 1e-6, "{what}: {got:?}, want {s:?} {e:?}");
    };
    for style in ["arc", "rise", "bulge"] {
        let warp = || Effect { id: format!("warp.{style}"), params: serde_json::json!({"bend": 40.0}), visible: true };
        let mut art = Node::group(NodeId(20), vec![sq(21, 0.0, Color::BLACK)]);
        art.appearance.items.push(AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(placed.clone())));
        let hook = move |_: &Node| Some(art.clone());
        let mut instance = Node::new(NodeId(1), NodeKind::SymbolInstance { symbol: "s".into(), xf: vectorcraft_geom::Affine::IDENTITY });
        instance.appearance.effects.push(warp());
        check(&shape_art(None, &instance, Some(&hook)).unwrap(), Rect::new(0.0, 0.0, 10.0, 10.0), &format!("warp.{style} on a symbol instance"));
        let mut shape = compound_shape(ShapeMode::Add);
        shape.appearance = Appearance::basic(placed.clone(), Paint::None, 0.0).into();
        shape.appearance.effects.push(warp());
        check(&shape_art(None, &shape, None).unwrap(), Rect::new(0.0, 0.0, 15.0, 10.0), &format!("warp.{style} on a compound shape"));
    }
}

#[test]
fn a_groups_transform_and_free_distort_move_the_gradients_placed_on_its_paths() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (0.0, 5.0).into(), end: (10.0, 5.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let ends = |e: Effect| {
        let square =
            Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(placed.clone(), Paint::None, 0.0)));
        let mut g = Node::group(NodeId(10), vec![square, sq(2, 20.0, Color::BLACK)]);
        g.appearance.effects.push(e);
        let art = evaluate_container(None, &g).unwrap();
        match art.children().unwrap()[0].appearance.fill_paint() {
            Paint::Gradient(g) => g.geom.map(|g| (g.start, g.end)).unwrap(),
            p => panic!("a gradient: {p:?}"),
        }
    };
    let near = |a: Point, b: Point| a.distance(b) < 1e-6;
    // A quarter turn about the group's center (15, 5) takes (0, 5) to (15, 20) and (10, 5) to (15, 10).
    let (s, e) = ends(Effect { id: "distort.transform".into(), params: serde_json::json!({"rotate": 90}), visible: true });
    assert!(near(s, Point::new(15.0, 20.0)) && near(e, Point::new(15.0, 10.0)), "{s:?} {e:?}");
    // Free Distort moving the group's top corners 10 pt right slants the square to 0..20 across:
    // its gradient follows its own bounds (0..10 to 0..20), not the group's (0..30 to 0..40).
    let corners = serde_json::json!({"corners": [[1.0 / 3.0, 0.0], [4.0 / 3.0, 0.0], [1.0, 1.0], [0.0, 1.0]]});
    let (s, e) = ends(Effect { id: "distort.freeDistort".into(), params: corners, visible: true });
    assert!(near(s, Point::new(0.0, 5.0)) && near(e, Point::new(20.0, 5.0)), "{s:?} {e:?}");
}

/// A group's Warp effect moves and scales the gradient placed on each of its paths from that path's
/// bounds before the warp to its bounds after it, not by the group's bounds.
#[test]
fn a_groups_warp_moves_the_gradients_placed_on_its_paths_by_each_paths_bounds() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = |x: f64| GradientGeom { start: (x + 2.0, 3.0).into(), end: (x + 8.0, 3.0).into(), aspect: 1.0, focal: None };
    let placed = |x: f64| Paint::Gradient(Box::new(GradientPaint { geom: Some(geom(x)), ..ramp.clone() }));
    let square = |id: u64, x: f64| {
        Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::basic(placed(x), Paint::None, 0.0)))
    };
    for style in ["arc", "bulge", "rise", "twist"] {
        let mut g = Node::group(NodeId(10), vec![square(1, 0.0), square(2, 20.0)]);
        g.appearance.effects.push(Effect { id: format!("warp.{style}"), params: serde_json::json!({"bend": 40.0}), visible: true });
        let art = evaluate_container(None, &g).unwrap();
        let mut moved = false;
        for (i, x) in [0.0, 20.0].into_iter().enumerate() {
            let m = &art.children().unwrap()[i];
            let b = m.geometric_bounds().unwrap();
            let follow = |p: Point| Point::new(b.x0 + (p.x - x) / 10.0 * b.width(), b.y0 + p.y / 10.0 * b.height());
            let Paint::Gradient(gp) = m.appearance.fill_paint() else { panic!("a gradient") };
            let got = gp.geom.unwrap();
            let want = geom(x);
            assert!(
                got.start.distance(follow(want.start)) < 1e-6 && got.end.distance(follow(want.end)) < 1e-6,
                "warp.{style} path {i}: {got:?} in {b:?}"
            );
            moved |= got.start.distance(want.start) > 0.1;
        }
        assert!(moved, "warp.{style} moves a gradient");
    }
}

/// Under a group's Warp effect, the group's own placed gradients follow the group's bounds, a
/// nested group's own placed gradients follow the nested group's bounds, and a compound path's
/// placed gradients follow the compound path's bounds, each from before the warp to after it (a
/// stroke's: those bounds inflated by the stroke).
#[test]
fn a_groups_warp_moves_its_own_a_nested_groups_and_a_compound_paths_gradients_by_their_bounds() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (12.0, 4.0).into(), end: (26.0, 4.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let own_fill = |n: &mut Node| {
        n.appearance.items.push(AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(placed.clone())));
        n.appearance.items.push(AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(placed.clone(), 3.0)));
    };
    let follow = |p: Point, from: Rect, to: Rect| {
        Point::new(to.x0 + (p.x - from.x0) / from.width() * to.width(), to.y0 + (p.y - from.y0) / from.height() * to.height())
    };
    for style in ["arc", "bulge", "rise", "flag"] {
        let mut inner = Node::group(NodeId(5), vec![sq(1, 0.0, Color::BLACK)]);
        own_fill(&mut inner);
        let ring = vec![
            Arc::new(Node::path(NodeId(6), shapes::rectangle(Rect::new(20.0, 0.0, 30.0, 14.0)), Appearance::default())),
            Arc::new(Node::path(NodeId(7), shapes::rectangle(Rect::new(23.0, 3.0, 27.0, 7.0)), Appearance::default())),
        ];
        let mut compound = Node::new(NodeId(3), NodeKind::Compound { children: ring, rule: FillRule::EvenOdd });
        compound.appearance = Appearance::basic(placed.clone(), placed.clone(), 2.0).into();
        let mut g = Node::group(NodeId(10), vec![Arc::new(inner), Arc::new(compound), sq(2, 40.0, Color::BLACK)]);
        own_fill(&mut g);
        let mut before = std::collections::HashMap::new();
        g.walk(&mut |c: &Node| {
            before.insert(c.id, c.geometric_bounds().unwrap());
        });
        g.appearance.effects.push(Effect { id: format!("warp.{style}"), params: serde_json::json!({"bend": 40.0}), visible: true });
        let art = evaluate_container(None, &g).unwrap();
        // The bounds after the warp of the group, the nested group and the compound path (the paths
        // a group's own fill paints carry the group's id).
        let mut after = std::collections::HashMap::new();
        art.walk(&mut |c: &Node| {
            if matches!(c.kind, NodeKind::Group { .. } | NodeKind::Compound { .. }) {
                after.entry(c.id).or_insert_with(|| c.geometric_bounds().unwrap());
            }
        });
        let mut checked = vec![];
        art.walk(&mut |c: &Node| {
            if ![NodeId(3), NodeId(5), NodeId(10)].contains(&c.id) {
                return;
            }
            for item in &c.appearance.items {
                let Paint::Gradient(gp) = item.paint() else { continue };
                let (from, to) = (before[&c.id], after[&c.id]);
                assert_ne!(from, to, "warp.{style} {:?}: the warp changes the bounds", c.id);
                let stroke = match item {
                    AppearanceItem::Stroke(s) => Some(s),
                    AppearanceItem::Fill(_) => None,
                };
                let (from, to) = match stroke {
                    Some(s) => (s.paint_bounds(from), s.paint_bounds(to)),
                    None => (from, to),
                };
                let got = gp.geom.unwrap();
                let (s, e) = (follow(geom.start, from, to), follow(geom.end, from, to));
                assert!(
                    got.start.distance(s) < 1e-6 && got.end.distance(e) < 1e-6,
                    "warp.{style} {:?} (stroke: {}): {got:?}, want {s:?} {e:?} ({from:?} to {to:?})",
                    c.id,
                    stroke.is_some()
                );
                checked.push((c.id, stroke.is_some()));
            }
        });
        checked.sort();
        checked.dedup();
        let want: Vec<_> = [NodeId(3), NodeId(5), NodeId(10)].into_iter().flat_map(|id| [(id, false), (id, true)]).collect();
        assert_eq!(checked, want, "warp.{style}");
    }
}

/// A group nested in a group with a Transform effect or Free Distort keeps its own placed gradient
/// on its members: the Transform's map moves it, and Free Distort moves and scales it from the
/// nested group's bounds before the effect to its bounds after it.
#[test]
fn a_nested_groups_own_gradient_follows_the_outer_groups_transform_and_free_distort() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (0.0, 5.0).into(), end: (10.0, 5.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let ends = |e: Effect| {
        let mut inner = Node::group(NodeId(5), vec![sq(1, 0.0, Color::BLACK)]);
        inner.appearance.items.push(AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(placed.clone())));
        let mut g = Node::group(NodeId(10), vec![Arc::new(inner), sq(2, 20.0, Color::BLACK)]);
        g.appearance.effects.push(e);
        let art = evaluate_container(None, &g).unwrap();
        match art.children().unwrap()[0].appearance.fill_paint() {
            Paint::Gradient(g) => g.geom.map(|g| (g.start, g.end)).unwrap(),
            p => panic!("a gradient: {p:?}"),
        }
    };
    let near = |a: Point, b: Point| a.distance(b) < 1e-6;
    let (s, e) = ends(Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 30}), visible: true });
    assert!(near(s, Point::new(30.0, 5.0)) && near(e, Point::new(40.0, 5.0)), "{s:?} {e:?}");
    let corners = serde_json::json!({"corners": [[1.0 / 3.0, 0.0], [4.0 / 3.0, 0.0], [1.0, 1.0], [0.0, 1.0]]});
    let (s, e) = ends(Effect { id: "distort.freeDistort".into(), params: corners, visible: true });
    assert!(near(s, Point::new(0.0, 5.0)) && near(e, Point::new(20.0, 5.0)), "{s:?} {e:?}");
}

/// An image or a symbol instance that a group's Transform effect moves takes the placed gradients
/// of its own fills along, as a path does, and each copy takes them as many times as it takes the
/// transform.
#[test]
fn a_groups_transform_moves_the_gradients_placed_on_an_images_own_fills() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    use vectorcraft_doc::ImageObject;
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (0.0, 5.0).into(), end: (10.0, 5.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    let image = || {
        NodeKind::Image(Box::new(ImageObject {
            key: "k".into(),
            width: 10,
            height: 10,
            xf: vectorcraft_geom::Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        }))
    };
    let instance = || NodeKind::SymbolInstance { symbol: "s".into(), xf: vectorcraft_geom::Affine::IDENTITY };
    let starts = |kind: NodeKind, params: serde_json::Value| {
        let mut object = Node::new(NodeId(1), kind);
        object.appearance = Appearance::basic(placed.clone(), Paint::None, 0.0).into();
        let mut g = Node::group(NodeId(10), vec![Arc::new(object), sq(2, 20.0, Color::BLACK)]);
        g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
        let art = evaluate_container(None, &g).unwrap();
        let mut out = vec![];
        art.walk(&mut |c: &Node| {
            if let (NodeKind::Image(_) | NodeKind::SymbolInstance { .. }, Paint::Gradient(p)) = (&c.kind, c.appearance.fill_paint()) {
                out.push(p.geom.unwrap().start.x);
            }
        });
        out
    };
    for kind in [image as fn() -> NodeKind, instance] {
        assert_eq!(starts(kind(), serde_json::json!({"moveH": 30})), [30.0], "{:?}", kind());
        assert_eq!(starts(kind(), serde_json::json!({"moveH": 30, "copies": 2})), [60.0, 30.0, 0.0], "{:?}", kind());
    }
}

/// A group's Transform effect that turns its art turns the gradients fitted to its paths, to the
/// group's own fill, to a nested group's own fill and to an image's and a symbol instance's own
/// fills: each is fixed to its fit on its own object first. A move leaves the members' gradients
/// fitted.
#[test]
fn a_groups_transform_turns_the_gradients_fitted_to_its_members_and_its_own_fill() {
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    use vectorcraft_doc::ImageObject;
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let fitted = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let square = |id: u64, x: f64| {
        Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::basic(fitted.clone(), Paint::None, 0.0)))
    };
    let mut image = Node::new(
        NodeId(4),
        NodeKind::Image(Box::new(ImageObject {
            key: "k".into(),
            width: 10,
            height: 10,
            xf: vectorcraft_geom::Affine::translate((30.0, 0.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    image.appearance = Appearance::basic(fitted.clone(), Paint::None, 0.0).into();
    // A symbol instance whose symbol is not in a document is measured as the 20 pt square about
    // its origin: here 22.5..27.5 across and 2.5..7.5 down.
    let at = vectorcraft_geom::Affine::translate((25.0, 5.0)) * vectorcraft_geom::Affine::scale(0.25);
    let mut instance = Node::new(NodeId(6), NodeKind::SymbolInstance { symbol: "s".into(), xf: at });
    instance.appearance = Appearance::basic(fitted.clone(), Paint::None, 0.0).into();
    // Ends of each gradient by object id; none for a gradient still fitted.
    let ends = |params: serde_json::Value| {
        let mut inner = Node::group(NodeId(5), vec![square(2, 10.0)]);
        inner.appearance.items.push(AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(fitted.clone())));
        let mut g = Node::group(NodeId(10), vec![square(1, 0.0), Arc::new(inner), Arc::new(image.clone()), Arc::new(instance.clone())]);
        g.appearance.items.push(AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(fitted.clone())));
        g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
        let art = evaluate_container(None, &g).unwrap();
        let mut out = std::collections::BTreeMap::new();
        art.walk(&mut |c: &Node| {
            if let Paint::Gradient(p) = c.appearance.fill_paint() {
                out.insert(c.id.0, p.geom.map(|g| (g.start, g.end)));
            }
        });
        out
    };
    let near = |a: Point, b: Point| a.distance(b) < 1e-6;
    // A quarter turn about the group's center (20, 5): (x, 5) goes to (20, 5 + 20 - x), so a ramp
    // from (x0, 5) to (x1, 5) runs from (20, 25 - x0) up to (20, 25 - x1).
    let turned = ends(serde_json::json!({"rotate": 90}));
    let want = [(1, 0.0, 10.0), (2, 10.0, 20.0), (5, 10.0, 20.0), (4, 30.0, 40.0), (6, 22.5, 27.5), (10, 0.0, 40.0)];
    for (id, x0, x1) in want {
        let got = turned.get(&id).copied().flatten();
        let ok = got.is_some_and(|(s, e)| near(s, Point::new(20.0, 25.0 - x0)) && near(e, Point::new(20.0, 25.0 - x1)));
        assert!(ok, "object {id}: {got:?} in {turned:?}");
    }
    // The group's own fill is fixed to its fit on the art it paints, which the move took to 30..70.
    let moved = ends(serde_json::json!({"moveH": 30}));
    let group = moved.get(&10).copied().flatten();
    assert!(group.is_some_and(|(s, e)| near(s, Point::new(30.0, 5.0)) && near(e, Point::new(70.0, 5.0))), "{moved:?}");
    assert!(moved.len() == 6 && [1, 2, 4, 5, 6].iter().all(|id| moved[id].is_none()), "{moved:?}");
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
        // The original is on top, after the copies below it.
        for (i, c) in copies.iter().rev().enumerate() {
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

/// A white 10 pt square with id `id` at the origin.
fn white_square(id: u64) -> Node {
    Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(Paint::solid(Color::WHITE), Paint::None, 0.0))
}

/// `n` masked by `art`, linked or not.
fn masked_by(mut n: Node, art: Node, linked: bool) -> Node {
    let mut mask = vectorcraft_doc::OpacityMask::new(art, true);
    mask.linked = linked;
    n.extras_mut().mask = Some(Box::new(mask));
    n
}

/// The geometry effects that reshape a group's members, one at a time.
fn reshaping_effects() -> Vec<Effect> {
    let fx = |id: &str, params: serde_json::Value| Effect { id: id.into(), params, visible: true };
    let corners = serde_json::json!({"corners": [[1.0 / 3.0, 0.0], [4.0 / 3.0, 0.0], [1.0, 1.0], [0.0, 1.0]]});
    vec![
        fx("distort.transform", serde_json::json!({"moveH": 30})),
        fx("distort.transform", serde_json::json!({"rotate": 90})),
        fx("distort.twist", serde_json::json!({"angle": 90})),
        fx("distort.freeDistort", corners),
        fx("warp.arc", serde_json::json!({})),
        fx("distort.roughen", serde_json::json!({"size": 5, "seed": 3})),
        fx("distort.zigZag", serde_json::json!({})),
        fx("distort.puckerBloat", serde_json::json!({"amount": 50})),
        fx("distort.tweak", serde_json::json!({})),
        fx("path.offsetPath", serde_json::json!({"offset": 3})),
        fx("stylize.roundCorners", serde_json::json!({"radius": 3})),
    ]
}

/// A group's geometry effect reshapes the art of a member's linked opacity mask as it reshapes the
/// member: a mask drawn over the member still covers it. An unlinked mask stays where it is.
#[test]
fn a_groups_geometry_effects_reshape_a_members_linked_mask_with_it() {
    let square = white_square(1);
    let original = square.path_data().cloned();
    for e in reshaping_effects() {
        for linked in [true, false] {
            let member = masked_by(square.clone(), white_square(4), linked);
            let mut g = Node::group(NodeId(10), vec![Arc::new(member), sq(2, 20.0, Color::BLACK)]);
            g.appearance.effects.push(e.clone());
            let art = evaluate_container(None, &g).unwrap();
            let member = &art.children().unwrap()[0];
            let mask = member.extras().mask.as_deref().unwrap();
            assert!(member.path_data().cloned() != original, "{}: the member is reshaped", e.id);
            let want = if linked { member.path_data().cloned() } else { original.clone() };
            assert_eq!(mask.art.path_data().cloned(), want, "{} (linked: {linked})", e.id);
            assert_eq!(mask.linked, linked);
        }
    }
}

/// The mask of a group inside the group, the mask of a path in it, a mask inside that mask's art and
/// an inverted mask all take the outer group's effect with the paths they mask.
#[test]
fn a_groups_geometry_effects_reshape_nested_and_inverted_masks() {
    for e in reshaping_effects() {
        let innermost = masked_by(white_square(7), white_square(8), true);
        let path = masked_by(white_square(1), masked_by(white_square(6), innermost, true), true);
        let mut inner = masked_by(Node::group(NodeId(5), vec![Arc::new(path)]), white_square(4), true);
        if let Some(m) = inner.extras_if_any().and_then(|x| x.mask.as_mut()) {
            m.invert = true;
        }
        let mut g = Node::group(NodeId(10), vec![Arc::new(inner), sq(2, 20.0, Color::BLACK)]);
        g.appearance.effects.push(e.clone());
        let art = evaluate_container(None, &g).unwrap();
        let inner = &art.children().unwrap()[0];
        let path = &inner.children().unwrap()[0];
        let want = path.path_data().cloned();
        let mask_of = |n: &Node| n.extras().mask.as_deref().map(|m| (*m.art).clone()).unwrap();
        let (outer_mask, path_mask) = (mask_of(inner), mask_of(path));
        assert!(inner.extras().mask.as_deref().is_some_and(|m| m.invert));
        assert_eq!(outer_mask.path_data().cloned(), want, "{}: the inner group's inverted mask", e.id);
        assert_eq!(path_mask.path_data().cloned(), want, "{}: the path's mask", e.id);
        assert_eq!(mask_of(&path_mask).path_data().cloned(), want, "{}: the mask inside the mask's art", e.id);
        assert_eq!(mask_of(&mask_of(&path_mask)).path_data().cloned(), want, "{}: one level deeper", e.id);
    }
}

/// A Transform effect's copies of a path with a linked mask are objects of their own, each with the
/// mask moved with it; the original keeps its mask where it was.
#[test]
fn each_transform_copy_of_a_masked_member_carries_its_mask() {
    let member = masked_by(white_square(1), white_square(4), true);
    let mut g = Node::group(NodeId(10), vec![Arc::new(member)]);
    let params = serde_json::json!({"moveH": 30, "copies": 2});
    g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
    let art = evaluate_container(None, &g).unwrap();
    let copies = art.children().unwrap()[0].children().expect("the object and its copies are objects of their own");
    assert_eq!(copies.len(), 3);
    for c in copies {
        let mask = c.extras().mask.as_deref().unwrap();
        assert_eq!(mask.art.geometric_bounds(), c.geometric_bounds());
    }
}

/// A Transform effect's copies of the paths of a member group give the group's linked mask the same
/// copies: the mask covers each copy.
#[test]
fn a_member_groups_mask_takes_the_transform_copies_of_its_paths() {
    let inner = masked_by(Node::group(NodeId(2), vec![Arc::new(white_square(1))]), white_square(4), true);
    let mut g = Node::group(NodeId(10), vec![Arc::new(inner)]);
    let params = serde_json::json!({"moveH": 30, "copies": 1});
    g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
    let art = evaluate_container(None, &g).unwrap();
    let inner = &art.children().unwrap()[0];
    let want = inner.geometric_bounds();
    assert_eq!(want, Some(Rect::new(0.0, 0.0, 40.0, 10.0)), "the path and its copy");
    assert_eq!(inner.extras().mask.as_deref().unwrap().art.geometric_bounds(), want);
}

/// A mask's art of several objects takes Transform copies whole, as the art it masks does: each copy
/// of the mask's art is a group of its objects, so the copies do not interleave object by object.
#[test]
fn a_masks_art_of_several_objects_is_copied_whole() {
    let mask_art = Node::group(NodeId(3), vec![Arc::new(white_square(4)), sq(5, 5.0, Color::WHITE)]);
    let inner = masked_by(Node::group(NodeId(2), vec![Arc::new(white_square(1))]), mask_art, true);
    let mut g = Node::group(NodeId(10), vec![Arc::new(inner)]);
    let params = serde_json::json!({"moveH": 30, "copies": 1});
    g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
    let art = evaluate_container(None, &g).unwrap();
    let mask = &art.children().unwrap()[0].extras().mask.as_deref().unwrap().art;
    let copies = mask.children().unwrap();
    assert_eq!(copies.len(), 2, "the mask's art and its copy");
    for c in copies {
        let ids: Vec<NodeId> = c.children().into_iter().flatten().map(|m| m.id).collect();
        assert_eq!(ids, [NodeId(4), NodeId(5)], "each copy holds the mask's objects");
    }
    assert_eq!(mask.geometric_bounds(), Some(Rect::new(0.0, 0.0, 45.0, 10.0)));
}

/// The linked mask of a mask's art that is copied whole takes the same Transform copies.
#[test]
fn the_mask_of_a_masks_art_copied_whole_takes_its_copies() {
    let parts = Node::group(NodeId(3), vec![Arc::new(white_square(4)), sq(5, 5.0, Color::WHITE)]);
    let mask_art = masked_by(parts, white_square(6), true);
    let inner = masked_by(Node::group(NodeId(2), vec![Arc::new(white_square(1))]), mask_art, true);
    let mut g = Node::group(NodeId(10), vec![Arc::new(inner)]);
    let params = serde_json::json!({"moveH": 30, "copies": 1});
    g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
    let art = evaluate_container(None, &g).unwrap();
    let mask = &art.children().unwrap()[0].extras().mask.as_deref().unwrap().art;
    assert_eq!(mask.geometric_bounds(), Some(Rect::new(0.0, 0.0, 45.0, 10.0)), "the mask's art and its copy");
    let own = &mask.extras().mask.as_deref().expect("the mask's art keeps its own mask").art;
    assert_eq!(own.geometric_bounds(), Some(Rect::new(0.0, 0.0, 40.0, 10.0)), "its mask covers the copy too");
}

/// The mask of a mask's art copied whole stays in the budget while that art is copied: when the
/// budget has no room for that mask's copy, the art keeps its shape with its masks.
#[test]
fn the_mask_of_a_masks_art_copied_whole_counts_against_the_budget() {
    let transform = Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 30, "copies": 1}), visible: true };
    let parts = Node::group(NodeId(3), vec![Arc::new(white_square(4)), sq(5, 5.0, Color::WHITE)]);
    let mask_art = masked_by(parts, white_square(6), true);
    let inner = masked_by(Node::group(NodeId(2), vec![Arc::new(white_square(1))]), mask_art, true);
    let mut art = Node::group(NodeId(10), vec![Arc::new(inner)]);
    let one = crate::Budget::of(white_square(1).path_data().unwrap()).lines;
    // Room for the four squares and the copies of three of them, not for the copy of the fourth.
    let budget = crate::Budget { anchors: usize::MAX / 4, lines: 7 * one };
    crate::reshape::reshape_leaves_within(&mut art, &[&transform], budget, None);
    let inner = &art.children().unwrap()[0];
    let square = Some(Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(inner.geometric_bounds(), square, "the path keeps its shape");
    let mask = &inner.extras().mask.as_deref().unwrap().art;
    assert_eq!(mask.geometric_bounds(), Some(Rect::new(0.0, 0.0, 15.0, 10.0)), "and so does its mask");
}

/// When a member group's linked mask can't take all the Transform copies its paths take (past the
/// budget), the art keeps its shape under the effect, so the mask still covers the paths.
#[test]
fn art_whose_mask_would_take_fewer_copies_keeps_its_shape() {
    let transform = Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 30, "copies": 1}), visible: true };
    let member = masked_by(Node::group(NodeId(2), vec![Arc::new(white_square(1))]), white_square(4), true);
    let mut art = Node::group(NodeId(10), vec![Arc::new(member)]);
    let one = crate::Budget::of(white_square(1).path_data().unwrap()).lines;
    // Room for the path, its mask and a copy of the path, not for a copy of the mask.
    let budget = crate::Budget { anchors: usize::MAX / 4, lines: 3 * one };
    crate::reshape::reshape_leaves_within(&mut art, &[&transform], budget, None);
    let member = &art.children().unwrap()[0];
    let square = Some(Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(member.geometric_bounds(), square, "the path keeps its shape");
    assert_eq!(member.extras().mask.as_deref().unwrap().art.geometric_bounds(), square, "and so does its mask");
}

/// The copies of a path with a linked mask count the art of the mask each copy carries against the
/// budget: they stop where the masks would pass it.
#[test]
fn transform_copies_count_the_art_of_their_masks() {
    let transform = Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 30, "copies": 2}), visible: true };
    let member = masked_by(white_square(1), white_square(4), true);
    let mut art = Node::group(NodeId(10), vec![Arc::new(member)]);
    let one = crate::Budget::of(white_square(1).path_data().unwrap()).lines;
    // Room for the path, its mask and one copy of both, not for a second copy.
    let budget = crate::Budget { anchors: usize::MAX / 4, lines: 4 * one };
    crate::reshape::reshape_leaves_within(&mut art, &[&transform], budget, None);
    let copies = art.children().unwrap()[0].children().expect("the object and its copy are objects of their own");
    assert_eq!(copies.len(), 2);
    for c in copies {
        assert_eq!(c.extras().mask.as_deref().unwrap().art.geometric_bounds(), c.geometric_bounds());
    }
}

/// A Transform effect that scales strokes moves a group's paths as objects, their masks with them.
/// The mask of a member group, whose art holds a path with a mask of its own, moves with the group's
/// paths too.
#[test]
fn a_transform_that_moves_paths_as_objects_moves_a_member_groups_nested_mask() {
    let mask_art = masked_by(white_square(4), white_square(5), true);
    let inner = masked_by(Node::group(NodeId(2), vec![Arc::new(white_square(1))]), mask_art, true);
    let mut g = Node::group(NodeId(10), vec![Arc::new(inner), sq(3, 20.0, Color::BLACK)]);
    let params = serde_json::json!({"moveH": 30, "strokes": true});
    g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
    let art = evaluate_container(None, &g).unwrap();
    let inner = &art.children().unwrap()[0];
    let want = inner.children().unwrap()[0].geometric_bounds();
    assert_eq!(want, Some(Rect::new(30.0, 0.0, 40.0, 10.0)));
    let mask = &inner.extras().mask.as_deref().unwrap().art;
    assert_eq!(mask.geometric_bounds(), want, "the group's mask");
    assert_eq!(mask.extras().mask.as_deref().unwrap().art.geometric_bounds(), want, "the mask inside its art");
}

/// A group's Transform effect moves an image member's linked opacity mask with the image, and its
/// copies' masks with the copies; an unlinked mask stays.
#[test]
fn a_groups_transform_moves_an_image_members_linked_mask() {
    for linked in [true, false] {
        for copies in [0, 2] {
            let im = vectorcraft_doc::ImageObject {
                key: "k".into(),
                width: 10,
                height: 10,
                xf: vectorcraft_geom::Affine::IDENTITY,
                link: None,
                placement: Default::default(),
                pixels: None,
            };
            let image = masked_by(Node::new(NodeId(1), NodeKind::Image(Box::new(im))), white_square(4), linked);
            let mut g = Node::group(NodeId(10), vec![Arc::new(image)]);
            let params = serde_json::json!({"moveH": 30, "copies": copies});
            g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
            let art = evaluate_container(None, &g).unwrap();
            let first = &art.children().unwrap()[0];
            let images: Vec<&Node> = match first.children() {
                Some(ch) => ch.iter().map(|c| &**c).collect(),
                None => vec![first],
            };
            assert_eq!(images.len(), copies + 1);
            for im in images {
                let mask = im.extras().mask.as_deref().unwrap();
                let want = if linked { im.geometric_bounds() } else { Some(Rect::new(0.0, 0.0, 10.0, 10.0)) };
                assert_eq!(mask.art.geometric_bounds(), want, "linked {linked}, copies {copies}");
            }
        }
    }
}

/// When a path in a member's mask art would pass the effects' budget, the whole mask stays as it
/// was, rather than part of its art moving with the member, and the member keeps its shape with it.
#[test]
fn a_mask_whose_art_passes_the_budget_stays_whole() {
    // Bloat bends a path's sides into curves drawn with more lines the larger they are: many more
    // on the mask's large square.
    let bloat = Effect { id: "distort.puckerBloat".into(), params: serde_json::json!({"amount": 50}), visible: true };
    let large = Node::path(NodeId(5), shapes::rectangle(Rect::new(0.0, 0.0, 1000.0, 1000.0)), Appearance::default());
    let mask_art = Node::group(NodeId(3), vec![Arc::new(white_square(4)), Arc::new(large.clone())]);
    let member = masked_by(white_square(1), mask_art.clone(), true);
    let mut art = Node::group(NodeId(10), vec![Arc::new(member)]);
    // Room for the member and the mask's small square to take the effect, not for the large one.
    let bounds = art.geometric_bounds().unwrap();
    let ctx = crate::GeomContext { seed: 10, ..crate::GeomContext::of(&art) };
    let square = white_square(1).path_data().cloned().unwrap();
    let made = crate::Budget::of(&crate::apply_geometry_with(std::slice::from_ref(&bloat), &square, bounds, &ctx)).lines;
    let budget = crate::Budget { anchors: usize::MAX / 4, lines: 2 * made + crate::Budget::of(large.path_data().unwrap()).lines };
    let large_made = crate::Budget::of(&crate::apply_geometry_with(std::slice::from_ref(&bloat), large.path_data().unwrap(), bounds, &ctx)).lines;
    assert!(large_made > crate::Budget::of(large.path_data().unwrap()).lines, "the large square's curves cost more lines");
    crate::reshape::reshape_leaves_within(&mut art, &[&bloat], budget, None);
    let member = &art.children().unwrap()[0];
    assert_eq!(member.path_data(), Some(&square), "the member keeps its shape with its mask");
    assert_eq!(*member.extras().mask.as_deref().unwrap().art, mask_art, "the mask stays whole");
}

/// Live objects in a member's linked mask are evaluated before the group's geometry effects run
/// over the mask's art, so the mask still covers the member: a Repeat in it moves and bends with
/// the member.
#[test]
fn a_groups_effects_reshape_a_live_object_in_a_members_mask() {
    let mut grid = vectorcraft_doc::RepeatSpec::grid(vec![Arc::new(white_square(5))], 0.0, 0.0);
    if let vectorcraft_doc::RepeatKind::Grid { rows, cols, .. } = &mut grid.kind {
        (*rows, *cols) = (1, 1);
    }
    let repeat = Node::new(NodeId(4), NodeKind::Repeat(grid));
    let drawn: Vec<Node> = crate::expand_live(None, &repeat);
    let leaf = |n: &Node| {
        let mut found = None;
        n.walk(&mut |c| found = found.take().or_else(|| c.path_data().cloned()));
        found
    };
    assert_eq!(drawn.iter().find_map(leaf), white_square(1).path_data().cloned(), "the Repeat draws one square over the member");
    for e in [reshaping_effects()[0].clone(), reshaping_effects()[4].clone()] {
        let member = masked_by(white_square(1), repeat.clone(), true);
        let mut g = Node::group(NodeId(10), vec![Arc::new(member), sq(2, 20.0, Color::BLACK)]);
        g.appearance.effects.push(e.clone());
        let art = evaluate_container(None, &g).unwrap();
        let member = &art.children().unwrap()[0];
        let mask = &member.extras().mask.as_deref().unwrap().art;
        assert!(member.path_data() != white_square(1).path_data(), "{}: the member is reshaped", e.id);
        assert_eq!(leaf(mask), member.path_data().cloned(), "{}: the mask's square takes the effect with it", e.id);
    }
}

/// Type in a member's linked mask is outlined before the group's Transform effect moves the mask's
/// art with the member.
#[test]
fn a_groups_transform_moves_type_in_a_members_mask() {
    if skip_without_craft_fonts() {
        return;
    }
    let member = masked_by(white_square(1), type_h(), true);
    let before = crate::reshape::outline_text(&type_h()).unwrap().geometric_bounds().unwrap();
    let mut g = Node::group(NodeId(10), vec![Arc::new(member)]);
    g.appearance.effects.push(reshaping_effects()[0].clone());
    let art = evaluate_container(None, &g).unwrap();
    let mask = &art.children().unwrap()[0].extras().mask.as_deref().unwrap().art;
    let mut text = false;
    mask.walk(&mut |c| text |= matches!(c.kind, NodeKind::Text(_)));
    assert!(!text, "the type is outlined");
    let after = mask.geometric_bounds().unwrap();
    assert!((after.x0 - before.x0 - 30.0).abs() < 0.5 && (after.y0 - before.y0).abs() < 0.5, "{before:?} -> {after:?}");
}

/// The linked mask of a compound path in a group, and of a path in a layer, takes the geometry
/// effects with the art it masks.
#[test]
fn a_compound_paths_and_a_layers_members_masks_take_the_effects() {
    let geometry = |n: &Node| crate::reshape::leaf_geometry(n).map(|(p, _)| p);
    for e in reshaping_effects() {
        let square = white_square(1);
        let compound = Node::new(NodeId(1), NodeKind::Compound { children: vec![Arc::new(square.clone())], rule: FillRule::NonZero });
        let compound = masked_by(compound, white_square(4), true);
        let mut g = Node::group(NodeId(10), vec![Arc::new(compound), sq(2, 20.0, Color::BLACK)]);
        g.appearance.effects.push(e.clone());
        let art = evaluate_container(None, &g).unwrap();
        let member = &art.children().unwrap()[0];
        assert!(geometry(member) != square.path_data().cloned(), "{}: the compound path is reshaped", e.id);
        assert_eq!(geometry(&member.extras().mask.as_deref().unwrap().art), geometry(member), "{}: compound path", e.id);

        let mut layer = Node::layer(NodeId(10), "Layer", vectorcraft_doc::LayerColor::Preset(0));
        if let Some(ch) = layer.children_mut() {
            ch.extend([Arc::new(masked_by(square.clone(), white_square(4), true)), sq(2, 20.0, Color::BLACK)]);
        }
        layer.appearance.effects.push(e.clone());
        let art = evaluate_container(None, &layer).unwrap();
        let member = &art.children().unwrap()[0];
        assert!(member.path_data() != square.path_data(), "{}: the layer's path is reshaped", e.id);
        assert_eq!(member.extras().mask.as_deref().unwrap().art.path_data(), member.path_data(), "{}: layer", e.id);
    }
}

/// A group's Transform effect moves a symbol instance member's linked mask with the instance.
#[test]
fn a_groups_transform_moves_a_symbol_instance_members_linked_mask() {
    let instance = Node::new(NodeId(1), NodeKind::SymbolInstance { symbol: "s".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let mut g = Node::group(NodeId(10), vec![Arc::new(masked_by(instance, white_square(4), true)), sq(2, 20.0, Color::BLACK)]);
    g.appearance.effects.push(reshaping_effects()[0].clone());
    let art = evaluate_container(None, &g).unwrap();
    let instance = &art.children().unwrap()[0];
    let NodeKind::SymbolInstance { xf, .. } = &instance.kind else { panic!("an instance: {instance:?}") };
    assert!((xf.translation().x - 30.0).abs() < 1e-9, "the instance moves: {xf:?}");
    let mask = instance.extras().mask.as_deref().unwrap().art.geometric_bounds().unwrap();
    assert!((mask.x0 - 30.0).abs() < 1e-9 && (mask.x1 - 40.0).abs() < 1e-9, "its mask moves with it: {mask:?}");
}

/// With Scale Strokes & Effects on, a group's Transform effect scales the stroke of an image
/// member's linked mask as it scales that of a path member's mask.
#[test]
fn a_transform_with_scale_strokes_scales_the_masks_of_images_and_paths_alike() {
    let stroked =
        Node::path(NodeId(4), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(Paint::None, Paint::solid(Color::WHITE), 2.0));
    let im = vectorcraft_doc::ImageObject {
        key: "k".into(),
        width: 10,
        height: 10,
        xf: vectorcraft_geom::Affine::IDENTITY,
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let image = masked_by(Node::new(NodeId(1), NodeKind::Image(Box::new(im))), stroked.clone(), true);
    let path = masked_by(white_square(2), stroked, true);
    let mut g = Node::group(NodeId(10), vec![Arc::new(image), Arc::new(path)]);
    let params = serde_json::json!({"scaleH": 200, "scaleV": 200, "strokes": true});
    g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
    let art = evaluate_container(None, &g).unwrap();
    let width = |n: &Node| n.extras().mask.as_deref().unwrap().art.appearance.stroke().map(|s| s.width);
    let ch = art.children().unwrap();
    assert_eq!(width(&ch[1]), Some(4.0), "the path's mask");
    assert_eq!(width(&ch[0]), width(&ch[1]), "the image's mask");
}

/// The art of a member group's mask is copied as it was before the first effect: a mask of one
/// path takes a second Transform effect's copies path by path, as the path it masks does, though
/// the first effect (with Scale Strokes & Effects) made it a group of copies.
#[test]
fn a_masks_art_is_copied_as_it_was_before_the_first_effect() {
    let inner = masked_by(Node::group(NodeId(2), vec![Arc::new(white_square(1))]), white_square(4), true);
    let mut g = Node::group(NodeId(10), vec![Arc::new(inner)]);
    for strokes in [true, false] {
        let params = serde_json::json!({"moveH": 30, "copies": 2, "strokes": strokes});
        g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
    }
    let art = evaluate_container(None, &g).unwrap();
    let inner = &art.children().unwrap()[0];
    let mask = &inner.extras().mask.as_deref().unwrap().art;
    assert_eq!(mask.geometric_bounds(), inner.geometric_bounds());
    let kinds = |n: &Node| n.children().unwrap().iter().map(|c| std::mem::discriminant(&c.kind)).collect::<Vec<_>>();
    let paths = &inner.children().unwrap()[0];
    assert_eq!(kinds(paths).len(), 3, "the first effect's three paths");
    assert!(paths.children().unwrap().iter().all(|c| matches!(c.kind, NodeKind::Compound { .. })), "each with its copies");
    assert_eq!(kinds(mask), kinds(paths), "the mask's art takes the copies as the paths do");
}

/// A 10 × 10 image with id `id` at the origin.
fn image_node(id: u64) -> Node {
    let im = vectorcraft_doc::ImageObject {
        key: "k".into(),
        width: 10,
        height: 10,
        xf: vectorcraft_geom::Affine::IDENTITY,
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    Node::new(NodeId(id), NodeKind::Image(Box::new(im)))
}

/// A linked mask whose art is an image moves with the path it masks under a group's Transform
/// effect, and a member group's image mask takes the copies its paths take.
#[test]
fn a_groups_transform_moves_a_linked_mask_whose_art_is_an_image() {
    let mut g = Node::group(NodeId(10), vec![Arc::new(masked_by(white_square(1), image_node(4), true)), sq(2, 20.0, Color::BLACK)]);
    g.appearance.effects.push(reshaping_effects()[0].clone());
    let art = evaluate_container(None, &g).unwrap();
    let member = &art.children().unwrap()[0];
    assert_eq!(member.geometric_bounds(), Some(Rect::new(30.0, 0.0, 40.0, 10.0)));
    assert_eq!(member.extras().mask.as_deref().unwrap().art.geometric_bounds(), member.geometric_bounds());

    let inner = masked_by(Node::group(NodeId(2), vec![Arc::new(white_square(1))]), image_node(4), true);
    let mut g = Node::group(NodeId(10), vec![Arc::new(inner)]);
    let params = serde_json::json!({"moveH": 30, "copies": 2});
    g.appearance.effects.push(Effect { id: "distort.transform".into(), params, visible: true });
    let art = evaluate_container(None, &g).unwrap();
    let inner = &art.children().unwrap()[0];
    assert_eq!(inner.geometric_bounds(), Some(Rect::new(0.0, 0.0, 70.0, 10.0)));
    let mask = &inner.extras().mask.as_deref().unwrap().art;
    assert_eq!(mask.geometric_bounds(), inner.geometric_bounds(), "the image and its copies");
}

/// An image in a linked mask's art takes no geometry effect but Transform, so under a Warp effect
/// the art keeps its shape with its mask.
#[test]
fn art_whose_mask_holds_an_image_keeps_its_shape_under_a_warp() {
    let member = masked_by(white_square(1), Node::group(NodeId(3), vec![Arc::new(image_node(4))]), true);
    let mut g = Node::group(NodeId(10), vec![Arc::new(member), sq(2, 20.0, Color::BLACK)]);
    g.appearance.effects.push(reshaping_effects()[4].clone());
    let art = evaluate_container(None, &g).unwrap();
    let member = &art.children().unwrap()[0];
    assert_eq!(member.path_data(), white_square(1).path_data(), "the path keeps its shape");
    assert_eq!(member.extras().mask.as_deref().unwrap().art.geometric_bounds(), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
}

/// A symbol instance in perspective projects the linked masks of the objects in its art with them.
#[test]
fn perspective_projects_the_masks_in_a_symbols_art() {
    let art = Node::group(NodeId(20), vec![Arc::new(masked_by(white_square(21), white_square(22), true))]);
    let mut instance = Node::new(NodeId(1), NodeKind::SymbolInstance { symbol: "s".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let mut at = vectorcraft_doc::PerspectiveAttachment::new("left", 0.0);
    at.projection = Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.0, 1.0]);
    instance.extras_mut().perspective = Some(Box::new(at));
    let out = crate::reshape::reshape(None, &instance, Some(&art)).unwrap();
    let mut masked = vec![];
    out.walk(&mut |c| masked.extend(c.extras().mask.as_deref().map(|m| (c.path_data().cloned(), m.art.path_data().cloned()))));
    let [(path, mask)] = masked.as_slice() else { panic!("one masked path: {masked:?}") };
    assert!(path != &white_square(21).path_data().cloned(), "the path is projected");
    assert_eq!(mask, path, "and so is its mask");
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
        let out = pathfinder_children(None, &g, None).unwrap();
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
        let out = pathfinder_children(None, &crossing_bars(n, "pathfinder.divide"), None).unwrap();
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
    let out = pathfinder_children(None, &g, None).unwrap();
    assert_eq!(out.len(), 1);
}

/// Two overlapping squares (0–10 and 5–15 by 10 high) as a compound shape, the front one in `mode`.
fn compound_shape(mode: ShapeMode) -> Node {
    let mut front = (*sq(2, 5.0, Color::rgb(0.0, 0.0, 1.0))).clone();
    front.extras_mut().shape_mode = mode;
    let mut g = Node::group(NodeId(10), vec![sq(1, 0.0, Color::rgb(1.0, 0.0, 0.0)), Arc::new(front)]);
    g.extras_mut().compound_shape = true;
    g.appearance = Appearance::basic(Paint::solid(Color::rgb(0.0, 1.0, 0.0)), Paint::None, 0.0).into();
    g
}

/// A compound shape draws the area its members' modes make, one unpainted path that its own
/// appearance paints; evaluated, it is plain art painted with that appearance.
#[test]
fn compound_shapes_draw_the_area_their_shape_modes_make() {
    for (mode, want) in [(ShapeMode::Add, 150.0), (ShapeMode::Subtract, 50.0), (ShapeMode::Intersect, 50.0), (ShapeMode::Exclude, 100.0)] {
        let g = compound_shape(mode);
        assert!(has_pathfinder(&g), "{mode:?}");
        let area_path = compound_shape_area(None, &g, None).unwrap();
        assert!((po::area(&area_path, FillRule::NonZero).abs() - want).abs() < 0.01, "{mode:?}");
        let out = pathfinder_children(None, &g, None).unwrap();
        assert_eq!(out.len(), 1, "{mode:?}");
        assert!(out[0].appearance.items.is_empty(), "the group's own appearance paints it");
        let art = evaluate_container(None, &g).unwrap();
        assert!(!art.is_compound_shape(), "evaluated, it is art");
        let painted: Vec<&Arc<Node>> = art.children().unwrap().iter().filter(|c| !c.appearance.items.is_empty()).collect();
        assert_eq!(painted.len(), 1);
        assert_eq!(painted[0].appearance.fill_paint().color(), Some(Color::rgb(0.0, 1.0, 0.0)), "{mode:?}");
        assert!((area(painted[0]) - want).abs() < 0.01, "{mode:?}");
    }
    // The backmost member's mode doesn't change the area.
    let mut g = compound_shape(ShapeMode::Add);
    if let Some(ch) = g.children_mut() {
        let mut back = (*ch[0]).clone();
        back.extras_mut().shape_mode = ShapeMode::Subtract;
        ch[0] = Arc::new(back);
    }
    assert!((po::area(&compound_shape_area(None, &g, None).unwrap(), FillRule::NonZero).abs() - 150.0).abs() < 0.01);
    // Hidden members take no part; the mark on a clip group is ignored.
    let mut g = compound_shape(ShapeMode::Subtract);
    if let Some(ch) = g.children_mut() {
        let mut front = (*ch[1]).clone();
        front.visible = false;
        ch[1] = Arc::new(front);
    }
    assert!((po::area(&compound_shape_area(None, &g, None).unwrap(), FillRule::NonZero).abs() - 100.0).abs() < 0.01);
    let mut clip = compound_shape(ShapeMode::Add);
    clip.kind = NodeKind::Group { children: clip.children().unwrap().clone(), clip: true };
    assert!(!clip.is_compound_shape() && compound_shape_area(None, &clip, None).is_none());
}

/// A compound shape inside another (and inside an Effect › Pathfinder group) takes part as its area,
/// styled like it.
#[test]
fn nested_compound_shapes_take_part_as_their_area() {
    let inner = compound_shape(ShapeMode::Subtract);
    let mut cutter = (*sq(3, 2.0, Color::BLACK)).clone();
    cutter.extras_mut().shape_mode = ShapeMode::Intersect;
    let mut outer = Node::group(NodeId(20), vec![Arc::new(inner.clone()), Arc::new(cutter)]);
    outer.extras_mut().compound_shape = true;
    // (0–5 of the inner area) ∩ (2–12) = 2–5 by 10 high.
    assert!((po::area(&compound_shape_area(None, &outer, None).unwrap(), FillRule::NonZero).abs() - 30.0).abs() < 0.01);
    let mut effect = Node::group(NodeId(30), vec![Arc::new(inner)]);
    effect.appearance.effects.push(Effect { id: "pathfinder.add".into(), params: serde_json::json!({}), visible: true });
    let out = pathfinder_children(None, &effect, None).unwrap();
    assert_eq!(out.len(), 1);
    assert!((area(&out[0]) - 50.0).abs() < 0.01);
    assert_eq!(out[0].appearance.fill_paint().color(), Some(Color::rgb(0.0, 1.0, 0.0)), "styled like the compound shape");
}

/// The ruler guides of a layer with a Pathfinder effect, a Transform effect that copies its art, or
/// fills and strokes of its own take no part: they stay last in the layer, once and unchanged.
#[test]
fn a_layers_ruler_guides_stay_last_through_its_effects() {
    use vectorcraft_doc::LayerColor;
    let guide = |id, pos| Arc::new(Node::new(NodeId(id), NodeKind::RulerGuide { vertical: true, pos, artboard: None }));
    let layer = |effect: &str, params: serde_json::Value| {
        let mut l = Node::layer(NodeId(10), "Layer 1", LayerColor::Preset(0));
        if let Some(ch) = l.children_mut() {
            *ch = vec![sq(1, 0.0, Color::rgb(1.0, 0.0, 0.0)), sq(2, 5.0, Color::rgb(0.0, 0.0, 1.0)), guide(20, 3.0), guide(21, 8.0)];
        }
        l.appearance.effects.push(Effect { id: effect.into(), params, visible: true });
        l
    };
    let tail = |n: &Node| {
        let ch = n.children().unwrap();
        assert_eq!(ch.iter().filter(|c| c.is_ruler_guide()).count(), 2, "each guide once");
        ch.iter().rev().take(2).map(|c| c.kind.clone()).collect::<Vec<_>>()
    };
    let guides = vec![guide(21, 8.0).kind.clone(), guide(20, 3.0).kind.clone()];

    let add = layer("pathfinder.add", serde_json::json!({}));
    let out = pathfinder_children(None, &add, None).unwrap();
    assert_eq!(out.len(), 3, "the united area, then the two guides");
    assert!((area(&out[0]) - 150.0).abs() < 0.01);
    assert!(out[1].is_ruler_guide() && out[2].is_ruler_guide());
    assert_eq!(tail(&evaluate_container(None, &add).unwrap()), guides);

    let copies = layer("distort.transform", serde_json::json!({"moveH": 30, "copies": 2}));
    let art = evaluate_container(None, &copies).unwrap();
    assert_eq!(tail(&art), guides);
    assert_eq!(art.children().unwrap().len(), 5, "three groups of the art, then the two guides");
    let mut art = copies.clone();
    crate::reshape::reshape_leaves(&mut art, &copies.appearance.effects.iter().collect::<Vec<_>>(), &copies.appearance.effects, None);
    assert_eq!(tail(&art), guides);
    assert_eq!(art.children().unwrap().len(), 5, "reshaped as members too");

    let mut painted = layer("distort.transform", serde_json::json!({"moveH": 30}));
    painted.appearance.items = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0).items;
    assert_eq!(tail(&evaluate_container(None, &painted).unwrap()), guides);
}

/// The ruler guides of a sublayer in a layer with a Pathfinder effect, or a Transform effect that
/// copies its art, stay on a layer of the evaluated art, once, hidden and locked as the sublayer is.
#[test]
fn a_sublayers_ruler_guides_survive_its_layers_effects() {
    use vectorcraft_doc::LayerColor;
    let layer = |effect: &str, params: serde_json::Value, shown: bool| {
        let mut sub = Node::layer(NodeId(11), "Sub", LayerColor::Preset(1));
        (sub.visible, sub.locked) = (shown, !shown);
        if let Some(ch) = sub.children_mut() {
            *ch = vec![
                sq(2, 5.0, Color::rgb(0.0, 0.0, 1.0)),
                Arc::new(Node::new(NodeId(20), NodeKind::RulerGuide { vertical: true, pos: 3.0, artboard: None })),
            ];
        }
        let mut l = Node::layer(NodeId(10), "Layer 1", LayerColor::Preset(0));
        if let Some(ch) = l.children_mut() {
            *ch = vec![sq(1, 0.0, Color::rgb(1.0, 0.0, 0.0)), Arc::new(sub)];
        }
        l.appearance.effects.push(Effect { id: effect.into(), params, visible: true });
        l
    };
    // The guides found where ruler guides are looked for: directly in a layer or sublayer.
    fn listed(n: &Node, out: &mut Vec<(NodeId, bool, bool)>) {
        for c in n.children().into_iter().flatten() {
            if c.is_ruler_guide() {
                out.push((c.id, c.visible, c.locked));
            } else if c.is_layer() {
                listed(c, out);
            }
        }
    }
    for (effect, params) in [("pathfinder.add", serde_json::json!({})), ("distort.transform", serde_json::json!({"moveH": 30, "copies": 2}))] {
        for shown in [true, false] {
            let art = evaluate_container(None, &layer(effect, params.clone(), shown)).unwrap();
            let mut found = vec![];
            listed(&art, &mut found);
            assert_eq!(found, [(NodeId(20), shown, !shown)], "{effect}: the sublayer's guide, once, on a layer");
        }
    }
}

/// A sublayer's ruler guide inside a hidden or locked group stays hidden or locked when its layer's
/// Pathfinder effect moves it onto the layer, as the group made it.
#[test]
fn a_sublayers_ruler_guide_in_a_hidden_or_locked_group_stays_hidden_or_locked() {
    use vectorcraft_doc::LayerColor;
    for (hidden, locked) in [(true, false), (false, true), (false, false)] {
        let mut sub = Node::layer(NodeId(11), "Sub", LayerColor::Preset(1));
        if let Some(ch) = sub.children_mut() {
            *ch = vec![
                sq(2, 5.0, Color::rgb(0.0, 0.0, 1.0)),
                Arc::new(Node::new(NodeId(20), NodeKind::RulerGuide { vertical: true, pos: 3.0, artboard: None })),
            ];
        }
        let mut group = Node::group(NodeId(12), vec![Arc::new(sub)]);
        (group.visible, group.locked) = (!hidden, locked);
        let mut l = Node::layer(NodeId(10), "Layer 1", LayerColor::Preset(0));
        if let Some(ch) = l.children_mut() {
            *ch = vec![sq(1, 0.0, Color::rgb(1.0, 0.0, 0.0)), Arc::new(group)];
        }
        l.appearance.effects.push(Effect { id: "pathfinder.add".into(), params: serde_json::json!({}), visible: true });
        let art = evaluate_container(None, &l).unwrap();
        let guides: Vec<_> = art.children().into_iter().flatten().filter(|c| c.is_ruler_guide()).map(|c| (c.id, c.visible, c.locked)).collect();
        assert_eq!(guides, [(NodeId(20), !hidden, locked)], "hidden {hidden}, locked {locked}");
    }
}
