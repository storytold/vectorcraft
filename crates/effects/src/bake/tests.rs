use super::*;
use serde_json::json;
use vectorcraft_color::{Color, Paint};
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

fn fx(id: &str, params: serde_json::Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
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
    let m = bake_appearance(&n).unwrap();
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
    let m = bake_appearance(&n).unwrap();
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
    let m = bake_appearance(&n).unwrap();
    let ch = m.children().unwrap();
    let NodeKind::Group { children, clip: true } = &ch[1].kind else { panic!("a clip group: {:?}", ch[1].kind) };
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    assert!(children.len() > 1 && children[1..].iter().all(|c| matches!(c.kind, NodeKind::Mesh(_))), "gradient meshes");
    assert_eq!(ch[1].opacity, 0.5);
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
