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
    // A clipping path that paints its fill keeps it, a path with the fill's effect live.
    let painted = first_clip(&crate::evaluate_container(None, &scaled(clip_with(Paint::solid(Color::BLACK)))).unwrap()).unwrap();
    assert!(painted.clip_paint().fill.is_some(), "{painted:?}");
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
