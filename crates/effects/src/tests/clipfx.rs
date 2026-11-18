//! Live effects on the clipping path of a clip group: the region it clips to takes the shape its
//! geometry effects give it, on the canvas as in an export.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeId, NodeKind};
use vectorcraft_geom::{Rect, Shape, shapes};

use crate::{bake_document, clip_outline};

fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

/// A 100 pt square clipping path at (50, 50) with `effects`.
fn clipping_path(id: u64, effects: Vec<Effect>) -> Node {
    let mut p = Node::path(NodeId(id), shapes::rectangle(Rect::new(50.0, 50.0, 150.0, 150.0)), Appearance::default());
    if let NodeKind::Path { clipping, .. } = &mut p.kind {
        *clipping = true;
    }
    p.appearance.effects = effects;
    p
}

/// A clip group: `clip` clipping a red 200 pt square at the origin.
fn clip_group(id: u64, clip: Node) -> Node {
    let art = Node::path(
        NodeId(id + 100),
        shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
    );
    Node::new(NodeId(id), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(art)], clip: true })
}

fn bounds(n: &Node) -> Rect {
    clip_outline(n).map(|(bp, _)| bp.bounding_box()).unwrap()
}

/// The clipping path of the first clip group in `doc` after an export bakes it.
fn baked_clip(doc: &Document) -> Arc<Node> {
    fn first(n: &Arc<Node>) -> Option<Arc<Node>> {
        match &n.kind {
            NodeKind::Group { children, clip: true } => children.first().cloned(),
            _ => n.children()?.iter().find_map(first),
        }
    }
    let baked = bake_document(doc).unwrap();
    baked.layers.iter().find_map(first).unwrap()
}

#[test]
fn transform_moves_the_clip() {
    let clip = clipping_path(1, vec![fx("distort.transform", json!({"moveH": 60}))]);
    let b = bounds(&clip);
    assert!((b.x0 - 110.0).abs() < 1e-6 && (b.x1 - 210.0).abs() < 1e-6, "the clip moves with the clipping path: {b:?}");
    assert!((b.y0 - 50.0).abs() < 1e-6 && (b.y1 - 150.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn roughen_clips_as_an_export_does() {
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    let group = clip_group(1, clipping_path(2, vec![fx("distort.roughen", json!({"size": 10, "detail": 10, "relative": false}))]));
    d.insert(Some(layer), 0, group.clone()).unwrap();
    let NodeKind::Group { children, .. } = &group.kind else { panic!("not a group") };
    let live = clip_outline(&children[0]).unwrap();
    let exported = clip_outline(&baked_clip(&d)).unwrap();
    assert_eq!(live.1, exported.1);
    assert_eq!(live.0, exported.0, "the canvas clips by the outline the export writes");
    assert_ne!(live.0.bounding_box(), Rect::new(50.0, 50.0, 150.0, 150.0), "the roughened outline, not the plain square");
}

#[test]
fn nested_members_keep_their_effects() {
    // A group as the clipping path: its member's Transform effect moves its part of the region.
    let moved = clipping_path(2, vec![fx("distort.transform", json!({"moveV": 60}))]);
    let group = Node::group(NodeId(1), vec![Arc::new(moved)]);
    let b = bounds(&group);
    assert!((b.y0 - 110.0).abs() < 1e-6 && (b.y1 - 210.0).abs() < 1e-6, "{b:?}");
    // A clip group as the clipping path clips by its own clipping path, effects and all.
    let nested = clip_group(3, clipping_path(4, vec![fx("distort.transform", json!({"moveH": -30}))]));
    let b = bounds(&nested);
    assert!((b.x0 - 20.0).abs() < 1e-6 && (b.x1 - 120.0).abs() < 1e-6, "{b:?}");
    // So does the nested clip group's own effect.
    let mut nested = clip_group(7, clipping_path(8, vec![]));
    nested.appearance.effects.push(fx("distort.transform", json!({"moveH": 30})));
    let b = bounds(&nested);
    assert!((b.x0 - 80.0).abs() < 1e-6 && (b.x1 - 180.0).abs() < 1e-6, "{b:?}");
    // The group's own Transform effect applies on top of its members'.
    let mut group = Node::group(NodeId(5), vec![Arc::new(clipping_path(6, vec![fx("distort.transform", json!({"moveV": 60}))]))]);
    group.appearance.effects.push(fx("distort.transform", json!({"moveH": 10})));
    let b = bounds(&group);
    assert!((b.x0 - 60.0).abs() < 1e-6 && (b.y0 - 110.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn a_roughened_group_clips_as_an_export_does() {
    // Two members side by side, Roughen on the group: each member is roughened, as in the export.
    let member = |id: u64, x: f64| {
        Arc::new(Node::path(
            NodeId(id),
            shapes::rectangle(Rect::new(x, 50.0, x + 40.0, 150.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        ))
    };
    let mut clip = Node::group(NodeId(2), vec![member(3, 50.0), member(4, 110.0)]);
    clip.appearance.effects.push(fx("distort.roughen", json!({"size": 10, "detail": 10, "relative": false})));
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, clip_group(1, clip.clone())).unwrap();
    let live = clip_outline(&clip).unwrap();
    assert_eq!(live.0, clip_outline(&baked_clip(&d)).unwrap().0, "the canvas clips by the outline the export writes");
    assert_ne!(live.0.bounding_box(), Rect::new(50.0, 50.0, 150.0, 150.0));
}

#[test]
fn a_compound_paths_member_keeps_its_effect() {
    // A ring whose hole a Transform effect moves 20 pt right, as the export bakes it.
    let ring = |effects: Vec<Effect>| {
        let outer = Node::path(NodeId(3), shapes::rectangle(Rect::new(50.0, 50.0, 150.0, 150.0)), Appearance::default());
        let mut hole = Node::path(NodeId(4), shapes::rectangle(Rect::new(75.0, 75.0, 125.0, 125.0)), Appearance::default());
        hole.appearance.effects = effects;
        Node::new(NodeId(2), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(hole)], rule: vectorcraft_geom::FillRule::EvenOdd })
    };
    let moved = ring(vec![fx("distort.transform", json!({"moveH": 20}))]);
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, clip_group(1, moved.clone())).unwrap();
    let live = clip_outline(&moved).unwrap();
    assert_eq!(live, clip_outline(&baked_clip(&d)).unwrap(), "the canvas clips by the outline the export writes");
    assert_ne!(live, clip_outline(&ring(vec![])).unwrap(), "the hole moves");
    // With a Transform of the compound path's own moving the whole ring 10 pt down after it.
    let mut both = moved.clone();
    both.appearance.effects.push(fx("distort.transform", json!({"moveV": 10})));
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, clip_group(1, both.clone())).unwrap();
    let live = clip_outline(&both).unwrap();
    assert_eq!(live, clip_outline(&baked_clip(&d)).unwrap(), "the canvas clips by the outline the export writes");
    assert_ne!(live.0.bounding_box(), clip_outline(&moved).unwrap().0.bounding_box(), "the ring moves down");
}

#[test]
fn the_clipping_paths_effects_apply_before_the_clip_groups() {
    // Scale 50 % on the clipping path, then Offset Path 10 on its clip group: the region is the
    // half-size square grown by 10, as if the clipping path had been drawn at half size.
    let group = |clip: Node| {
        let mut g = clip_group(1, clip);
        g.appearance.effects.push(fx("path.offsetPath", json!({"offset": 10})));
        g
    };
    let scaled = group(clipping_path(2, vec![fx("distort.transform", json!({"scaleH": 50, "scaleV": 50}))]));
    let mut half = clipping_path(2, vec![]);
    if let NodeKind::Path { path, .. } = &mut half.kind {
        *path = shapes::rectangle(Rect::new(75.0, 75.0, 125.0, 125.0));
    }
    let want = bounds(&group(half));
    let got = bounds(&scaled);
    assert!((got.x0 - want.x0).abs() < 0.5 && (got.x1 - want.x1).abs() < 0.5, "{got:?} against {want:?}");
    assert!((want.x0 - 65.0).abs() < 0.5, "{want:?}");
    // The clip group as the canvas evaluates it for its effect, and as an export bakes it.
    let evaluated = crate::evaluate_container(None, &scaled).unwrap();
    let NodeKind::Group { children, clip: true } = &evaluated.kind else { panic!("not a clip group") };
    let b = clip_outline(&children[0]).unwrap().0.bounding_box();
    assert!((b.x0 - want.x0).abs() < 0.5 && (b.x1 - want.x1).abs() < 0.5, "evaluated: {b:?} against {want:?}");
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, scaled).unwrap();
    let b = clip_outline(&baked_clip(&d)).unwrap().0.bounding_box();
    assert!((b.x0 - want.x0).abs() < 0.5 && (b.x1 - want.x1).abs() < 0.5, "exported: {b:?} against {want:?}");
}

#[test]
fn a_clip_group_with_a_fill_or_stroke_of_its_own_clips_as_the_canvas_does() {
    // A clip group with a fill or a stroke of its own and no effects: the export turns its paint
    // into art and still clips by the outline the canvas clips by, Transform copies included.
    let items = [
        vectorcraft_doc::AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(Paint::solid(Color::BLACK))),
        vectorcraft_doc::AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 2.0)),
    ];
    let copies = || fx("distort.transform", json!({"moveH": 60, "copies": 1}));
    let roughen = fx("distort.roughen", json!({"size": 10, "detail": 10, "relative": false}));
    // A ring: a compound path of two squares with effects `own`, its hole with effects `hole_fx`.
    let ring = |hole_fx: Vec<Effect>, own: Vec<Effect>| {
        let outer = Node::path(NodeId(3), shapes::rectangle(Rect::new(50.0, 50.0, 150.0, 150.0)), Appearance::default());
        let mut hole = Node::path(NodeId(4), shapes::rectangle(Rect::new(75.0, 75.0, 125.0, 125.0)), Appearance::default());
        hole.appearance.effects = hole_fx;
        let mut c =
            Node::new(NodeId(2), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(hole)], rule: vectorcraft_geom::FillRule::EvenOdd });
        c.appearance.effects = own;
        c
    };
    let clips = [
        ("copies", clipping_path(2, vec![copies()])),
        ("roughen", clipping_path(2, vec![roughen])),
        ("a compound path's copies", ring(vec![], vec![copies()])),
        ("a compound path member's copies", ring(vec![copies()], vec![])),
    ];
    for item in &items {
        for (what, clip) in &clips {
            let mut d = Document::new(400.0, 400.0);
            let layer = d.layers[0].id;
            let live = clip_outline(clip).unwrap();
            let mut group = clip_group(1, clip.clone());
            group.appearance.items.push(item.clone());
            d.insert(Some(layer), 0, group).unwrap();
            let exported = clip_outline(&baked_clip(&d)).unwrap();
            assert_eq!(live, exported, "{what} with {item:?}: the canvas clips by the outline the export writes");
        }
    }
    let b = bounds(&clips[0].1);
    assert!((b.x0 - 50.0).abs() < 1e-6 && (b.x1 - 210.0).abs() < 1e-6, "the clip covers both copies: {b:?}");
}

#[test]
fn nested_effects_apply_before_the_clip_groups() {
    // The clip group grows its clipping path by 10 (Offset Path). Inside the clipping path a
    // Transform moves a member 60 pt right: the region grows from the moved member.
    let evaluated = |clip: Node| {
        let mut g = clip_group(1, clip);
        g.appearance.effects.push(fx("path.offsetPath", json!({"offset": 10})));
        let e = crate::evaluate_container(None, &g).unwrap();
        let NodeKind::Group { children, clip: true } = &e.kind else { panic!("not a clip group") };
        clip_outline(&children[0]).unwrap().0.bounding_box()
    };
    let moved = || clipping_path(3, vec![fx("distort.transform", json!({"moveH": 60}))]);
    let close = |a: Rect, b: Rect| (a.x0 - b.x0).abs() < 0.5 && (a.x1 - b.x1).abs() < 0.5 && (a.y0 - b.y0).abs() < 0.5;
    let want = Rect::new(100.0, 40.0, 220.0, 160.0);
    // A group holding the member.
    let b = evaluated(Node::group(NodeId(2), vec![Arc::new(moved())]));
    assert!(close(b, want), "group: {b:?}");
    // A compound path of the member, with a Transform of its own moving it 10 pt down.
    let mut compound = Node::new(NodeId(2), NodeKind::Compound { children: vec![Arc::new(moved())], rule: vectorcraft_geom::FillRule::NonZero });
    compound.appearance.effects.push(fx("distort.transform", json!({"moveV": 10})));
    let b = evaluated(compound);
    assert!(close(b, Rect::new(100.0, 50.0, 220.0, 170.0)), "compound: {b:?}");
}

#[test]
fn clipping_type_keeps_its_effect_where_other_type_stays_type() {
    // Type moved 30 pt right by its Transform clips a red square, beside other type, in a group
    // that a Transform moves 10 pt down: the export keeps the other type as type and outlines the
    // clipping type, which still clips where the canvas clips.
    use vectorcraft_doc::text::{CharStyle, TextObject};
    let text = |id: u64, effects: Vec<Effect>| {
        let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
        let mut t = Node::new(NodeId(id), NodeKind::Text(Box::new(TextObject::point(vectorcraft_geom::Point::new(5.0, 60.0), "IIII", style))));
        t.appearance.effects = effects;
        t
    };
    let clipped = clip_group(2, text(3, vec![fx("distort.transform", json!({"moveH": 30}))]));
    let mut outer = Node::group(NodeId(1), vec![Arc::new(clipped), Arc::new(text(5, vec![]))]);
    outer.appearance.effects.push(fx("distort.transform", json!({"moveV": 10})));
    fn clip_of(n: &Node) -> Option<Rect> {
        match &n.kind {
            NodeKind::Group { children, clip: true } => Some(clip_outline(children.first()?)?.0.bounding_box()),
            _ => n.children()?.iter().find_map(|c| clip_of(c)),
        }
    }
    if vectorcraft_text::CRAFT_FONTS.is_empty() {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        return;
    }
    let plain = clip_outline(&text(3, vec![])).unwrap().0.bounding_box();
    let canvas = clip_of(&crate::evaluate_container(None, &outer).unwrap()).unwrap();
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, outer).unwrap();
    let baked = bake_document(&d).unwrap();
    let mut exported = None;
    for l in &baked.layers {
        exported = exported.or_else(|| clip_of(l));
    }
    let exported = exported.unwrap();
    assert!((canvas.x0 - plain.x0 - 30.0).abs() < 0.5 && (canvas.y0 - plain.y0 - 10.0).abs() < 0.5, "{canvas:?} from {plain:?}");
    assert!((exported.x0 - canvas.x0).abs() < 0.5 && (exported.y0 - canvas.y0).abs() < 0.5, "exported {exported:?}, canvas {canvas:?}");
    // The outlined clipping type keeps its black fill, which paints behind the clipped art.
    let outlined = crate::reshape::clipping_text_outline(&text(3, vec![fx("distort.transform", json!({"moveH": 30}))])).unwrap();
    fn fills(n: &Node) -> usize {
        n.appearance.items.iter().filter(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(f) if f.paint != Paint::None)).count()
            + n.children().into_iter().flatten().map(|c| fills(c)).sum::<usize>()
    }
    assert!(fills(&outlined) > 0, "{outlined:?}");
}

#[test]
fn a_painted_clipping_paths_gradient_follows_its_effect() {
    // A clipping path filled with a gradient placed across it, moved 60 pt right by its Transform
    // effect, in a clip group with an effect of its own: the gradient moves with the path.
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::WHITE)];
    let geom = GradientGeom { start: (50.0, 100.0).into(), end: (150.0, 100.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..GradientPaint::new(Gradient::new(GradientKind::Linear, stops)) }));
    let mut clip = clipping_path(2, vec![fx("distort.transform", json!({"moveH": 60}))]);
    clip.appearance = Appearance { effects: clip.appearance.effects.clone(), ..Appearance::basic(placed, Paint::None, 0.0) }.into();
    let mut group = clip_group(1, clip);
    group.appearance.effects.push(fx("path.offsetPath", json!({"offset": 0})));
    let evaluated = crate::evaluate_container(None, &group).unwrap();
    let NodeKind::Group { children, clip: true } = &evaluated.kind else { panic!("not a clip group") };
    let Paint::Gradient(g) = children[0].appearance.fill_paint() else { panic!("no gradient") };
    let start = g.geom.unwrap().start;
    assert!((start.x - 110.0).abs() < 1e-6, "the gradient starts at the moved path's left edge: {start:?}");
}

#[test]
fn a_nested_clip_groups_hidden_clipping_path_keeps_its_effect() {
    // A clip group used as a clipping path clips by its clipping path whether it is shown or not.
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    let mut hidden = clipping_path(4, vec![fx("distort.transform", json!({"moveH": -30}))]);
    hidden.visible = false;
    let outer = clip_group(1, clip_group(3, hidden));
    d.insert(Some(layer), 0, outer.clone()).unwrap();
    let NodeKind::Group { children, .. } = &outer.kind else { panic!("not a group") };
    let b = bounds(&children[0]);
    assert!((b.x0 - 20.0).abs() < 1e-6 && (b.x1 - 120.0).abs() < 1e-6, "{b:?}");
    assert_eq!(clip_outline(&children[0]), clip_outline(&baked_clip(&d)), "the canvas clips by the outline the export writes");
}

#[test]
fn a_fills_geometry_effect_shapes_the_clip_as_an_export_does() {
    // The clipping path's fill has a Transform effect moving it 60 pt right; the path has none.
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    let mut clip = clipping_path(2, vec![]);
    clip.appearance = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 0.0)), Paint::None, 0.0).into();
    for item in &mut clip.appearance.items {
        if let vectorcraft_doc::AppearanceItem::Fill(f) = item {
            f.effects.push(fx("distort.transform", json!({"moveH": 60})));
        }
    }
    let group = clip_group(1, clip);
    d.insert(Some(layer), 0, group.clone()).unwrap();
    let NodeKind::Group { children, .. } = &group.kind else { panic!("not a group") };
    let live = clip_outline(&children[0]).unwrap();
    let exported = clip_outline(&baked_clip(&d)).unwrap();
    assert_eq!(live.0.bounding_box(), exported.0.bounding_box(), "the canvas clips by the outline the export writes");
    assert!((live.0.bounding_box().x1 - 210.0).abs() < 1e-6, "the moved fill reaches 210: {:?}", live.0.bounding_box());
    // A path of a compound clipping path with a fill effect of its own stays part of the region.
    let mut member = clipping_path(4, vec![]);
    member.appearance = children[0].appearance.clone();
    let other = Node::path(NodeId(5), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default());
    let compound =
        Node::new(NodeId(3), NodeKind::Compound { children: vec![Arc::new(member), Arc::new(other)], rule: vectorcraft_geom::FillRule::NonZero });
    assert_eq!(bounds(&compound), Rect::new(0.0, 0.0, 150.0, 150.0));
}

#[test]
fn a_fills_geometry_effect_shapes_the_clip_before_the_effects_around_it() {
    // Offset Path 10 on the fill of an unpainted clipping path, in a group that its Transform
    // scales to 50 %: the region is the grown square at half size, 60 pt across, as it is when the
    // clipping path is the member of a group that clips by its members.
    let clip_with = |fill: Paint| {
        let mut clip = clipping_path(3, vec![]);
        clip.appearance = Appearance::basic(fill, Paint::None, 0.0).into();
        for item in &mut clip.appearance.items {
            if let vectorcraft_doc::AppearanceItem::Fill(f) = item {
                f.effects.push(fx("path.offsetPath", json!({"offset": 10})));
            }
        }
        clip
    };
    let scaled = |clip: Node| {
        let mut outer = Node::group(NodeId(1), vec![Arc::new(clip_group(2, clip))]);
        outer.appearance.effects.push(fx("distort.transform", json!({"scaleH": 50, "scaleV": 50})));
        outer
    };
    fn first_clip(n: &Node) -> Option<Arc<Node>> {
        match &n.kind {
            NodeKind::Group { children, clip: true } => children.first().cloned(),
            _ => n.children()?.iter().find_map(|c| first_clip(c)),
        }
    }
    let clip_of = |n: &Node| clip_outline(&first_clip(n).unwrap()).unwrap().0.bounding_box();
    let close = |r: Rect| (r.width() - 60.0).abs() < 0.5 && (r.height() - 60.0).abs() < 0.5;
    let nested = clip_of(&crate::evaluate_container(None, &scaled(Node::group(NodeId(4), vec![Arc::new(clip_with(Paint::None))]))).unwrap());
    assert!(close(nested), "nested: {nested:?}");
    let outer = scaled(clip_with(Paint::None));
    let canvas = clip_of(&crate::evaluate_container(None, &outer).unwrap());
    assert!(close(canvas), "canvas: {canvas:?}");
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, outer).unwrap();
    let baked = bake_document(&d).unwrap();
    let exported = baked.layers.iter().find_map(|l| first_clip(l)).map(|c| clip_outline(&c).unwrap().0.bounding_box()).unwrap();
    assert!(close(exported), "exported: {exported:?}");
    // A clipping path that paints its fill clips alike and paints the fill it clips by.
    let painted = first_clip(&crate::evaluate_container(None, &scaled(clip_with(Paint::solid(Color::BLACK)))).unwrap()).unwrap();
    assert!(close(clip_outline(&painted).unwrap().0.bounding_box()), "painted: {painted:?}");
    let fill = painted.clip_paint().fill.unwrap();
    assert!(close(fill.geometric_bounds().unwrap()), "painted fill: {fill:?}");
}

#[test]
fn a_painted_clipping_paths_fill_and_stroke_effects_apply_before_the_effects_around_it() {
    // A clipping path with a black fill grown by Offset Path 10 and a 2 pt blue stroke moved 30 pt
    // right by a Transform, in a group that its Transform scales to 50 %: the fill paints the grown
    // square at half size, 60 pt across, and the stroke the square at half size, 15 pt right of
    // it, on the canvas and in an export alike.
    let mut clip = clipping_path(3, vec![]);
    clip.appearance = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 2.0).into();
    for item in &mut clip.appearance.items {
        match item {
            vectorcraft_doc::AppearanceItem::Fill(f) => f.effects.push(fx("path.offsetPath", json!({"offset": 10}))),
            vectorcraft_doc::AppearanceItem::Stroke(s) => s.effects.push(fx("distort.transform", json!({"moveH": 30}))),
        }
    }
    let mut outer = Node::group(NodeId(1), vec![Arc::new(clip_group(2, clip.clone()))]);
    outer.appearance.effects.push(fx("distort.transform", json!({"scaleH": 50, "scaleV": 50})));
    fn first_clip(n: &Node) -> Option<Arc<Node>> {
        match &n.kind {
            NodeKind::Group { children, clip: true } => children.first().cloned(),
            _ => n.children()?.iter().find_map(|c| first_clip(c)),
        }
    }
    let check = |what: &str, c: &Node| {
        let paint = c.clip_paint();
        let fill = paint.fill.as_ref().and_then(Node::geometric_bounds).unwrap();
        assert!((fill.width() - 60.0).abs() < 0.5 && (fill.height() - 60.0).abs() < 0.5, "{what} fill: {fill:?}");
        let stroke = paint.stroke.as_ref().and_then(Node::geometric_bounds).unwrap();
        assert!((stroke.width() - 50.0).abs() < 0.5 && (stroke.x0 - fill.x0 - 20.0).abs() < 0.5, "{what} stroke: {stroke:?} fill: {fill:?}");
    };
    let evaluated = crate::evaluate_container(None, &outer).unwrap();
    let canvas = first_clip(&evaluated).unwrap();
    check("canvas", &canvas);
    // The clip group's box takes in the stroke's width, as for a path that paints its stroke.
    let stroke = canvas.clip_paint().stroke.as_ref().and_then(Node::geometric_bounds).unwrap();
    let visual = evaluated.children().and_then(|ch| ch.first()).and_then(|g| g.visual_bounds()).unwrap();
    assert!(visual.x1 > stroke.x1 + 0.25, "{visual:?} {stroke:?}");
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, outer).unwrap();
    check("exported", &baked_clip(&d));
    // Without effects around it, the export paints the fill and the stroke as the canvas does.
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, clip_group(2, clip.clone())).unwrap();
    let paint = baked_clip(&d).clip_paint();
    let fill = paint.fill.as_ref().and_then(Node::geometric_bounds);
    let stroke = paint.stroke.as_ref().and_then(Node::geometric_bounds);
    assert_eq!(fill.map(|f| f.width()), Some(120.0), "{paint:?}");
    assert_eq!(stroke.map(|s| s.x0), Some(80.0), "{paint:?}");
    // A piece that an effect around it splits into several paths (a compound path), or puts in a
    // group with its copies, still paints.
    let NodeKind::Group { children, .. } = &canvas.kind else { panic!("not the pieces: {canvas:?}") };
    let mut split = canvas.as_ref().clone();
    let mut compound = children[0].as_ref().clone();
    compound.kind = NodeKind::Compound { children: vec![children[0].clone(), children[0].clone()], rule: vectorcraft_geom::FillRule::NonZero };
    let copies = Node::group(NodeId(9), vec![children[1].clone(), children[1].clone()]);
    if let Some(ch) = split.children_mut() {
        ch[0] = Arc::new(compound);
        ch[1] = Arc::new(copies);
    }
    let paint = split.clip_paint();
    assert!(paint.fill.is_some() && paint.stroke.is_some(), "{split:?}");
    let stroke = paint.stroke.unwrap();
    assert_eq!(stroke.children().map(|ch| ch.len()), Some(1), "only the stroke's group: {stroke:?}");
}

#[test]
fn a_painted_clipping_paths_stroke_keeps_the_width_its_transform_scales_it_to() {
    // A compound clipping path with a 2 pt stroke whose Transform scales strokes to 200 % without
    // transforming the object: the export paints the stroke at 4 pt.
    let square = Node::path(NodeId(4), shapes::rectangle(Rect::new(50.0, 50.0, 150.0, 150.0)), Appearance::default());
    let mut clip = Node::new(NodeId(3), NodeKind::Compound { children: vec![Arc::new(square)], rule: vectorcraft_geom::FillRule::NonZero });
    clip.appearance = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 2.0).into();
    let scale = json!({"scaleH": 200, "scaleV": 200, "objects": false, "strokes": true});
    for item in &mut clip.appearance.items {
        if let vectorcraft_doc::AppearanceItem::Stroke(s) = item {
            s.effects.push(fx("distort.transform", scale.clone()));
        }
    }
    fn widths(n: &Node, out: &mut Vec<f64>) {
        for item in &n.appearance.items {
            if let vectorcraft_doc::AppearanceItem::Stroke(s) = item {
                out.push(s.width);
            }
        }
        n.children().into_iter().flatten().for_each(|c| widths(c, out));
    }
    let mut d = Document::new(400.0, 400.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, clip_group(2, clip)).unwrap();
    let baked = baked_clip(&d);
    let stroke = baked.clip_paint().stroke.unwrap();
    let mut found = vec![];
    widths(&stroke, &mut found);
    assert!(!found.is_empty() && found.iter().all(|w| (w - 4.0).abs() < 1e-6), "{found:?} {stroke:?}");
    // The stroke is still drawn where the path lies, as the Transform does not move the object.
    let bounds = stroke.geometric_bounds().unwrap();
    assert!((bounds.x0 - 50.0).abs() < 0.5 && (bounds.width() - 100.0).abs() < 0.5, "{bounds:?}");
}

#[test]
fn a_baked_clipping_path_gives_its_pieces_ids_of_their_own() {
    // An unpainted clipping path whose fill and stroke have Transform effects, in a clip group
    // without effects: the export bakes it into a piece for each, every object with its own id.
    let mut d = Document::new(400.0, 400.0);
    let (group, path) = (d.alloc_id(), d.alloc_id());
    let mut clip = clipping_path(path.0, vec![]);
    clip.appearance = Appearance::basic(Paint::None, Paint::None, 1.0).into();
    for item in &mut clip.appearance.items {
        match item {
            vectorcraft_doc::AppearanceItem::Fill(f) => f.effects.push(fx("distort.transform", json!({"moveH": 60}))),
            vectorcraft_doc::AppearanceItem::Stroke(s) => s.effects.push(fx("distort.transform", json!({"moveV": 60}))),
        }
    }
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, clip_group(group.0, clip)).unwrap();
    let baked = bake_document(&d).unwrap();
    let clip = baked_clip(&d);
    assert_eq!(clip.children().map(|c| c.len()), Some(2), "{clip:?}");
    fn ids(n: &Node, out: &mut Vec<NodeId>) {
        out.push(n.id);
        for c in n.children().into_iter().flatten() {
            ids(c, out);
        }
    }
    let mut all = vec![];
    for l in &baked.layers {
        ids(l, &mut all);
    }
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(unique.len(), all.len(), "{all:?}");
}

#[test]
fn a_hidden_member_with_an_effect_adds_nothing() {
    let shown = Node::path(NodeId(3), shapes::rectangle(Rect::new(50.0, 50.0, 100.0, 100.0)), Appearance::default());
    let mut hidden = clipping_path(4, vec![fx("distort.transform", json!({"moveH": 200}))]);
    hidden.visible = false;
    let group = Node::group(NodeId(2), vec![Arc::new(shown), Arc::new(hidden)]);
    assert_eq!(bounds(&group), Rect::new(50.0, 50.0, 100.0, 100.0));
}

#[test]
fn raster_and_hidden_effects_leave_the_clip_as_it_is() {
    let mut hidden = fx("distort.transform", json!({"moveH": 60}));
    hidden.visible = false;
    for effects in [vec![], vec![fx("blur.gaussian", json!({"radius": 10}))], vec![hidden]] {
        assert_eq!(bounds(&clipping_path(1, effects)), Rect::new(50.0, 50.0, 150.0, 150.0));
    }
}

fn blue() -> Paint {
    Paint::solid(Color::rgb(0.0, 0.0, 1.0))
}

fn green() -> Paint {
    Paint::solid(Color::rgb(0.0, 1.0, 0.0))
}

/// "IIII" at 40 pt from (5, 60) in a test font built from the bundled Source Sans 3
/// ([`vectorcraft_text::test_fonts::height_font`]), its characters filled blue when `fill` and
/// stroked green 2 pt wide when `stroke`.
fn painted_type(id: u64, fill: bool, stroke: bool) -> Node {
    use vectorcraft_doc::text::{CharStyle, TextObject};
    use vectorcraft_text::test_fonts::{HEIGHT_FAMILY, height_font};
    static FONT: std::sync::Once = std::sync::Once::new();
    FONT.call_once(|| {
        vectorcraft_text::FontDb::global().add_font(height_font(true).expect("the test font builds"));
    });
    let style = CharStyle {
        font_family: HEIGHT_FAMILY.into(),
        size: 40.0,
        fill: if fill { blue() } else { Paint::None },
        stroke: if stroke { green() } else { Paint::None },
        stroke_width: 2.0,
        ..Default::default()
    };
    Node::new(NodeId(id), NodeKind::Text(Box::new(TextObject::point(vectorcraft_geom::Point::new(5.0, 60.0), "IIII", style))))
}

/// Does `n` or an object in it have a blue fill (`fill`) or a green stroke?
fn paints(n: &Node, fill: bool) -> bool {
    use vectorcraft_doc::AppearanceItem;
    let own = n.appearance.items.iter().any(|i| match i {
        AppearanceItem::Fill(f) => fill && f.paint == blue(),
        AppearanceItem::Stroke(s) => !fill && s.paint == green() && s.width > 0.0,
    });
    own || n.children().into_iter().flatten().any(|c| paints(c, fill))
}

/// What the first clip group in `n` paints: the bounds of the region it clips to, the bounds of
/// the blue fill art right after its clipping path (behind the clipped art), the bounds of the
/// green stroke art right after the clip group (over it, outside the clip), and whether a green
/// stroke paints inside the clip group.
type Painted = (Rect, Option<Rect>, Option<Rect>, bool);

fn painted_clip(n: &Node) -> Option<Painted> {
    let ch = n.children()?;
    for (i, c) in ch.iter().enumerate() {
        if let NodeKind::Group { children, clip: true } = &c.kind {
            let region = clip_outline(children.first()?)?.0.bounding_box();
            let fill = children.get(1).filter(|f| paints(f, true)).and_then(|f| f.geometric_bounds());
            let stroke = ch.get(i + 1).filter(|s| paints(s, false)).and_then(|s| s.geometric_bounds());
            let inside = children.iter().skip(1).any(|m| paints(m, false));
            return Some((region, fill, stroke, inside));
        }
        if let Some(found) = painted_clip(c) {
            return Some(found);
        }
    }
    None
}

fn near_rect(a: Rect, b: Rect) -> bool {
    (a.x0 - b.x0).abs() < 0.5 && (a.y0 - b.y0).abs() < 0.5 && (a.x1 - b.x1).abs() < 0.5 && (a.y1 - b.y1).abs() < 0.5
}

#[test]
fn painted_clipping_type_survives_enclosing_geometry_effect() {
    // Type that paints a fill, a stroke, both or neither clips a red square, in a group whose
    // Transform (moving everything 10 pt down, beside other type that stays type in the export) or
    // Offset Path reshapes its members: the canvas and the export keep the type's fill as art
    // behind the square inside the clip and its stroke as art over the clip group, both reshaped
    // with the clip.
    let plain = clip_outline(&painted_type(3, false, false)).expect("the type clips").0.bounding_box();
    assert!(plain.width() > 10.0 && plain.height() > 10.0, "the type has glyph outlines: {plain:?}");
    let effects = [fx("distort.transform", json!({"moveV": 10})), fx("path.offsetPath", json!({"offset": 1}))];
    for effect in effects {
        for (fill, stroke) in [(true, false), (false, true), (true, true), (false, false)] {
            let clipped = clip_group(2, painted_type(3, fill, stroke));
            let mut outer = Node::group(NodeId(1), vec![Arc::new(clipped), Arc::new(painted_type(5, true, false))]);
            outer.appearance.effects.push(effect.clone());
            let canvas = crate::evaluate_container(None, &outer).unwrap();
            let mut d = Document::new(400.0, 400.0);
            let layer = d.layers[0].id;
            d.insert(Some(layer), 0, outer).unwrap();
            let baked = bake_document(&d).unwrap();
            let exported = baked.layers.iter().find_map(|l| painted_clip(l));
            for (what, found) in [("canvas", painted_clip(&canvas)), ("export", exported)] {
                let case = format!("{what}, {}, fill {fill}, stroke {stroke}", effect.id);
                let (region, fills, strokes, inside) = found.unwrap_or_else(|| panic!("{case}: no clip group"));
                if effect.id == "distort.transform" {
                    assert!((region.y0 - plain.y0 - 10.0).abs() < 0.5 && (region.x0 - plain.x0).abs() < 0.5, "{case}: {region:?} from {plain:?}");
                } else {
                    let grown = Rect::new(plain.x0 - 1.0, plain.y0 - 1.0, plain.x1 + 1.0, plain.y1 + 1.0);
                    assert!(near_rect(region, grown), "{case}: {region:?} is {plain:?} offset by 1 pt");
                }
                assert_eq!(fills.is_some(), fill, "{case}: fill art {fills:?}");
                assert_eq!(strokes.is_some(), stroke, "{case}: stroke art {strokes:?}");
                assert!(!inside, "{case}: the stroke paints outside the clip");
                for art in fills.into_iter().chain(strokes) {
                    assert!(near_rect(art, region), "{case}: the paint {art:?} follows the clip {region:?}");
                }
            }
        }
    }
}

/// The group in `n` that holds a clip group and then the green stroke art of its clipping type,
/// with that clip group.
fn stroke_wrapper(n: &Node) -> Option<(&Node, &Node)> {
    let ch = n.children()?;
    if let [inner, stroke, ..] = ch.as_slice()
        && matches!(inner.kind, NodeKind::Group { clip: true, .. })
        && !n.clips()
        && paints(stroke, false)
    {
        return Some((n, inner));
    }
    ch.iter().find_map(|c| stroke_wrapper(c))
}

#[test]
fn clipping_type_strokes_paint_over_a_knockout_clip_group() {
    // A clip group set to Knockout Group On, Off or neutral, clipped by type that strokes: its
    // clipped art knocks out as the clip group says, and the type's stroke paints over it without
    // knocking it out, as the canvas draws the live clip group. The type is reshaped by a group's
    // Transform around it, or by its own Offset Path.
    use vectorcraft_doc::Knockout;
    for (knockout, around) in [(Knockout::On, Knockout::Off), (Knockout::Off, Knockout::Off), (Knockout::Neutral, Knockout::Neutral)] {
        for own in [false, true] {
            let mut ty = painted_type(3, true, true);
            if own {
                ty.appearance.effects.push(fx("path.offsetPath", json!({"offset": 1})));
            }
            let mut clipped = clip_group(2, ty);
            clipped.knockout = knockout;
            let mut outer = Node::group(NodeId(1), vec![Arc::new(clipped)]);
            if !own {
                outer.appearance.effects.push(fx("distort.transform", json!({"moveV": 10})));
            }
            let mut d = Document::new(400.0, 400.0);
            let layer = d.layers[0].id;
            d.insert(Some(layer), 0, outer.clone()).unwrap();
            let baked = bake_document(&d).unwrap();
            let canvas = if own { None } else { crate::evaluate_container(None, &outer) };
            let exported = baked.layers.iter().find_map(|l| stroke_wrapper(l));
            for (what, found) in [("canvas", canvas.as_ref().and_then(stroke_wrapper)), ("export", exported)] {
                if own && what == "canvas" {
                    continue;
                }
                let case = format!("{what}, own effect {own}, knockout {knockout:?}");
                let (wrapper, inner) = found.unwrap_or_else(|| panic!("{case}: no group around the clip"));
                assert_eq!(inner.knockout, knockout, "{case}: the clipped art knocks out as before");
                assert_eq!(wrapper.knockout, around, "{case}: the stroke does not knock out the clipped art");
            }
        }
    }
}

#[test]
fn a_clip_groups_own_fill_stays_in_the_clip_when_its_clipping_type_strokes() {
    // A clip group with a yellow fill of its own and an Offset Path effect, clipped by type that
    // strokes: the group's fill still paints inside the clip, and the type's stroke over it.
    let yellow = Paint::solid(Color::rgb(1.0, 1.0, 0.0));
    let mut group = clip_group(2, painted_type(3, false, true));
    group.appearance =
        Appearance { effects: vec![fx("path.offsetPath", json!({"offset": 0}))], ..Appearance::basic(yellow.clone(), Paint::None, 0.0) }.into();
    let canvas = crate::evaluate_container(None, &group).unwrap();
    let (_, _, strokes, inside) = painted_clip(&Node::group(NodeId(9), vec![Arc::new(canvas.clone())])).unwrap();
    assert!(strokes.is_some() && !inside, "{canvas:?}");
    let NodeKind::Group { children, clip: false } = &canvas.kind else { panic!("not a group around the clip: {canvas:?}") };
    let NodeKind::Group { children: clipped, clip: true } = &children[0].kind else { panic!("no clip group first: {children:?}") };
    let yellow_in = |n: &Node| n.appearance.items.iter().any(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(f) if f.paint == yellow));
    assert!(clipped.iter().skip(1).any(|c| yellow_in(c)), "the group's fill paints inside the clip: {clipped:?}");
    assert!(!children.iter().skip(1).any(|c| yellow_in(c)), "{children:?}");
}

#[test]
fn a_clip_groups_own_gradient_spans_the_clip_when_its_clipping_type_strokes() {
    // A clip group with an unplaced linear gradient fill of its own and an Offset Path effect,
    // clipped by type with a stroke whose Transform effect moves it 60 pt right: the gradient
    // spans the region the clipping path clips to.
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::WHITE)];
    let unplaced = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let mut ty = painted_type(3, false, false);
    ty.appearance = Appearance::basic(Paint::None, green(), 2.0).into();
    for item in &mut ty.appearance.items {
        if let vectorcraft_doc::AppearanceItem::Stroke(s) = item {
            s.effects.push(fx("distort.transform", json!({"moveH": 60})));
        }
    }
    let mut group = clip_group(2, ty);
    group.appearance =
        Appearance { effects: vec![fx("path.offsetPath", json!({"offset": 0}))], ..Appearance::basic(unplaced, Paint::None, 0.0) }.into();
    let canvas = crate::evaluate_container(None, &group).unwrap();
    let NodeKind::Group { children, clip: false } = &canvas.kind else { panic!("not a group around the clip: {canvas:?}") };
    let NodeKind::Group { children: clipped, clip: true } = &children[0].kind else { panic!("no clip group first: {children:?}") };
    let region = clip_outline(&clipped[0]).expect("the type clips").0.bounding_box();
    let fill = clipped.iter().skip(1).find_map(|c| match c.appearance.fill_paint() {
        Paint::Gradient(g) => g.geom,
        _ => None,
    });
    let geom = fill.unwrap_or_else(|| panic!("no gradient fill inside the clip: {clipped:?}"));
    let (left, right) = (geom.start.x.min(geom.end.x), geom.start.x.max(geom.end.x));
    assert!((left - region.x0).abs() < 0.5 && (right - region.x1).abs() < 0.5, "the gradient {geom:?} spans the clip {region:?}");
}

#[test]
fn a_groups_own_fill_paints_only_the_clip_of_clipping_type_whose_stroke_moves() {
    // A group with a fill of its own (solid, or an unplaced linear gradient) and a Transform effect
    // (moving everything 10 pt down, or scaling it to half its width around its middle) holds a
    // clip group clipped by type whose stroke has a Transform effect moving it 60 pt right,
    // outside the clip. On the canvas and in the export, the clip region is the type's glyphs
    // reshaped by the group's effect, and the group's fill paints that region alone, its gradient
    // spanning it: the moved stroke adds nothing to the region, the fill or the box the group's
    // effect scales around.
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let yellow = Paint::solid(Color::rgb(1.0, 1.0, 0.0));
    let stops = vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::WHITE)];
    let unplaced = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let mut ty = painted_type(3, false, false);
    ty.appearance = Appearance::basic(Paint::None, green(), 2.0).into();
    for item in &mut ty.appearance.items {
        if let vectorcraft_doc::AppearanceItem::Stroke(s) = item {
            s.effects.push(fx("distort.transform", json!({"moveH": 60})));
        }
    }
    let live = clip_outline(&ty).expect("the type clips").0.bounding_box();
    let moved = Rect::new(live.x0, live.y0 + 10.0, live.x1, live.y1 + 10.0);
    // Scaled to half its width around the middle of the clip, which the stroke does not move.
    let (mid, quarter) = ((live.x0 + live.x1) / 2.0, live.width() / 4.0);
    let narrowed = Rect::new(mid - quarter, live.y0, mid + quarter, live.y1);
    // The group's own fill art: every object outside the clip groups whose own fill is `paint`.
    fn own_fills<'a>(n: &'a Node, paint: &Paint, out: &mut Vec<&'a Node>) {
        let mine = n.appearance.items.iter().any(|i| match (i, paint) {
            (vectorcraft_doc::AppearanceItem::Fill(f), Paint::Gradient(_)) => matches!(f.paint, Paint::Gradient(_)),
            (vectorcraft_doc::AppearanceItem::Fill(f), p) => f.paint == *p,
            _ => false,
        });
        if mine {
            out.push(n);
        }
        for c in n.children().into_iter().flatten().filter(|c| !c.clips()) {
            own_fills(c, paint, out);
        }
    }
    let cases = [(json!({"moveV": 10}), moved), (json!({"scaleH": 50, "scaleV": 100}), narrowed)];
    for ((params, region), paint) in cases.iter().flat_map(|c| [yellow.clone(), unplaced.clone()].map(|p| (c.clone(), p))) {
        let mut outer = Node::group(NodeId(1), vec![Arc::new(clip_group(2, ty.clone()))]);
        outer.appearance =
            Appearance { effects: vec![fx("distort.transform", params.clone())], ..Appearance::basic(paint.clone(), Paint::None, 0.0) }.into();
        let canvas = crate::evaluate_container(None, &outer).unwrap();
        let mut d = Document::new(400.0, 400.0);
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, outer).unwrap();
        let baked = bake_document(&d).unwrap();
        let exported = baked.layers[0].children().and_then(|ch| ch.first()).cloned().expect("the export holds the group");
        for (what, art) in [("canvas", &canvas), ("export", &*exported)] {
            let case = format!("{what}, {params}, {}", if matches!(paint, Paint::Gradient(_)) { "gradient" } else { "solid" });
            let (clip, _, strokes, _) = painted_clip(art).unwrap_or_else(|| panic!("{case}: no clip group in {art:?}"));
            assert!(near_rect(clip, region), "{case}: the clip {clip:?} is the type's glyphs reshaped {region:?}");
            let strokes = strokes.unwrap_or_else(|| panic!("{case}: no stroke art"));
            assert!(strokes.x1 > region.x1 + 25.0, "{case}: the stroke {strokes:?} moves outside the clip {region:?}");
            let mut fills = vec![];
            own_fills(art, &paint, &mut fills);
            assert!(!fills.is_empty(), "{case}: no fill art of the group in {art:?}");
            let painted = fills.iter().filter_map(|f| f.geometric_bounds()).reduce(|a, b| a.union(b)).unwrap();
            assert!(near_rect(painted, region), "{case}: the group's fill {painted:?} paints the clip {region:?}");
            if let Paint::Gradient(_) = paint {
                for fill in fills {
                    let geom = match fill.appearance.fill_paint() {
                        Paint::Gradient(g) => g.geom,
                        _ => None,
                    };
                    let geom = geom.unwrap_or_else(|| panic!("{case}: the gradient is not placed: {fill:?}"));
                    let (left, right) = (geom.start.x.min(geom.end.x), geom.start.x.max(geom.end.x));
                    assert!(
                        (left - region.x0).abs() < 0.5 && (right - region.x1).abs() < 0.5,
                        "{case}: the gradient {geom:?} spans the clip {region:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_clip_groups_placed_gradient_follows_free_distort_by_the_clip_when_its_clipping_type_strokes() {
    // A clip group with a placed linear gradient fill of its own, in a group with Free Distort,
    // clipped by type whose stroke has a Transform effect moving it 60 pt right, outside the clip:
    // the gradient moves and scales from the clip's bounds, as for the same clip group whose type
    // does not stroke. The moved stroke adds nothing to those bounds.
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::WHITE)];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: (10.0, 45.0).into(), end: (30.0, 45.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..ramp }));
    fn placed_geom(n: &Node) -> Option<vectorcraft_color::GradientGeom> {
        if let Paint::Gradient(g) = n.appearance.fill_paint()
            && g.geom.is_some()
        {
            return g.geom;
        }
        n.children().into_iter().flatten().find_map(|c| placed_geom(c))
    }
    let ends = |stroke: bool| {
        let mut ty = painted_type(3, false, false);
        if stroke {
            ty.appearance = Appearance::basic(Paint::None, green(), 2.0).into();
            for item in &mut ty.appearance.items {
                if let vectorcraft_doc::AppearanceItem::Stroke(s) = item {
                    s.effects.push(fx("distort.transform", json!({"moveH": 60})));
                }
            }
        }
        let mut group = clip_group(2, ty);
        group.appearance = Appearance::basic(placed.clone(), Paint::None, 0.0).into();
        let mut outer = Node::group(NodeId(1), vec![Arc::new(group)]);
        let corners = json!({"corners": [[1.0 / 3.0, 0.0], [4.0 / 3.0, 0.0], [1.0, 1.0], [0.0, 1.0]]});
        outer.appearance.effects.push(fx("distort.freeDistort", corners));
        let canvas = crate::evaluate_container(None, &outer).unwrap();
        if stroke {
            assert!(stroke_wrapper(&canvas).is_some(), "the type's stroke paints around the clip: {canvas:?}");
        }
        placed_geom(&canvas).unwrap_or_else(|| panic!("no placed gradient: {canvas:?}"))
    };
    let (plain, stroked) = (ends(false), ends(true));
    assert!(
        plain.start.distance(stroked.start) < 0.5 && plain.end.distance(stroked.end) < 0.5,
        "the gradient {stroked:?} follows the clip as {plain:?} does"
    );
}
