//! The appearance of groups and layers.
//!
//! **Effect → Pathfinder**: live Pathfinder operations on a group (or layer). The group's direct
//! children are the Pathfinder stack (back → front). In the shape modes (Add, Subtract, Intersect,
//! Exclude, Minus Back) a child group acts as one shape — the union of its leaves, styled like its
//! top leaf — as in the Pathfinder panel. The result replaces the children at render time; each
//! result keeps the appearance of the object whose paint it takes.
//!
//! **Container appearance** ([`evaluate_container`]): a group's or layer's own fills and strokes
//! paint every member's geometry, below or above the members as its contents slot says
//! ([`Appearance::contents_at`]); its geometry effects reshape the members as one piece; its raster
//! effects (one combined shadow) apply to the composite and stay for the renderer and exporters.

use std::sync::Arc;

use vectorcraft_doc::{Appearance, AppearanceItem, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData, Rect};
use vectorcraft_pathops::{self as po, PathfinderOp};

/// Effect ids of the Pathfinder submenu with their operation, in Illustrator's menu order.
pub const PATHFINDER_EFFECTS: [(&str, &str, PathfinderOp); 10] = [
    ("pathfinder.add", "Add", PathfinderOp::Unite),
    ("pathfinder.intersect", "Intersect", PathfinderOp::Intersect),
    ("pathfinder.exclude", "Exclude", PathfinderOp::Exclude),
    ("pathfinder.subtract", "Subtract", PathfinderOp::MinusFront),
    ("pathfinder.minusBack", "Minus Back", PathfinderOp::MinusBack),
    ("pathfinder.divide", "Divide", PathfinderOp::Divide),
    ("pathfinder.trim", "Trim", PathfinderOp::Trim),
    ("pathfinder.merge", "Merge", PathfinderOp::Merge),
    ("pathfinder.crop", "Crop", PathfinderOp::Crop),
    ("pathfinder.outline", "Outline", PathfinderOp::Outline),
];

/// Is `id` an Effect → Pathfinder effect?
pub fn is_pathfinder(id: &str) -> bool {
    id.starts_with("pathfinder.")
}

fn op_of(id: &str) -> Option<PathfinderOp> {
    PATHFINDER_EFFECTS.iter().find(|e| e.0 == id).map(|e| e.2)
}

/// Does `n` (a group or layer) carry a visible Pathfinder effect?
pub fn has_pathfinder(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. } | NodeKind::Layer { .. })
        && n.appearance.effects.iter().any(|e| e.visible && is_pathfinder(&e.id))
}

/// Converts objects that aren't paths (e.g. text) to outline art, if possible. The paths and
/// compound paths in that art, at every depth of its groups, take part in the Pathfinder operation
/// with the object's opacity and blend mode.
pub type OutlineHook<'a> = &'a dyn Fn(&Node) -> Option<Node>;

fn leaves(n: &Node, hook: Option<OutlineHook>, out: &mut Vec<Node>) {
    if !n.visible {
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => out.push(n.clone()),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children {
                leaves(c, hook, out);
            }
        }
        _ => {
            // Every path of the outline art, at any depth, takes the object's opacity and blend
            // mode.
            if let Some(art) = hook.and_then(|h| h(n)) {
                let first = out.len();
                leaves(&art, None, out);
                for l in out.iter_mut().skip(first) {
                    (l.opacity, l.blend) = (n.opacity, n.blend);
                }
            }
        }
    }
}

fn geometry(n: &Node) -> Option<(PathData, FillRule)> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule))
        }
        _ => None,
    }
}

/// A result path styled like `src` (its appearance, opacity and blend mode).
fn styled(src: &Node, path: PathData, outline: bool) -> Node {
    let mut appearance = Appearance { items: src.appearance.items.clone(), ..Default::default() };
    if outline {
        let fill = src.appearance.fill_paint();
        let paint = if fill.is_none() { src.appearance.stroke_paint() } else { fill };
        appearance = Appearance { items: vec![AppearanceItem::Stroke(StrokeLayer::new(paint, 1.0))], ..Default::default() };
    }
    let mut n = Node::path(src.id, path, appearance);
    n.opacity = src.opacity;
    n.blend = src.blend;
    n
}

/// Evaluate the visible Pathfinder effects of `group` in order: the resulting child objects.
/// `None` when the group has no Pathfinder effect.
pub fn pathfinder_children(group: &Node, hook: Option<OutlineHook>) -> Option<Vec<Arc<Node>>> {
    if !has_pathfinder(group) {
        return None;
    }
    let mut stack: Vec<Arc<Node>> = group.children()?.to_vec();
    for e in group.appearance.effects.iter().filter(|e| e.visible) {
        let Some(op) = op_of(&e.id) else { continue };
        let shape_mode =
            matches!(op, PathfinderOp::Unite | PathfinderOp::MinusFront | PathfinderOp::Intersect | PathfinderOp::Exclude | PathfinderOp::MinusBack);
        // (style source, shape) per stack entry; keys index `sources`.
        let mut sources: Vec<Node> = vec![];
        let mut shapes: Vec<po::Shape> = vec![];
        for c in &stack {
            let mut lv = vec![];
            leaves(c, hook, &mut lv);
            if shape_mode && lv.len() > 1 {
                let parts: Vec<(PathData, FillRule)> = lv.iter().filter_map(geometry).collect();
                let refs: Vec<(&PathData, FillRule)> = parts.iter().map(|(p, r)| (p, *r)).collect();
                let key = sources.len() as u64;
                if let Some(top) = lv.pop() {
                    shapes.push(po::Shape::new(po::unite_all(&refs), FillRule::NonZero, key));
                    sources.push(top);
                }
            } else {
                for l in lv {
                    if let Some((p, r)) = geometry(&l) {
                        shapes.push(po::Shape::new(p, r, sources.len() as u64));
                        sources.push(l);
                    }
                }
            }
        }
        shapes.retain(|s| !s.path.is_empty() && s.path.subpaths.iter().all(|sp| sp.anchors.iter().all(|a| a.p.x.is_finite() && a.p.y.is_finite())));
        if shapes.is_empty() {
            return Some(vec![]);
        }
        stack = po::pathfinder(op, &shapes)
            .into_iter()
            .filter(|r| !r.path.is_empty())
            .map(|r| Arc::new(styled(&sources[r.key as usize], r.path, op == PathfinderOp::Outline)))
            .collect();
    }
    Some(stack)
}

fn is_container(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. })
}

/// Does `item` paint anything?
pub fn paints(item: &AppearanceItem) -> bool {
    item.visible()
        && !item.paint().is_none()
        && match item {
            AppearanceItem::Stroke(s) => s.width > 0.0,
            AppearanceItem::Fill(_) => true,
        }
}

/// Do the own fills or strokes of `n`, a group or layer, paint anything?
pub(crate) fn has_own_paint(n: &Node) -> bool {
    is_container(n) && n.appearance.items.iter().any(paints)
}

/// Does `n`, a group or layer, have an appearance of its own to evaluate: a fill or stroke that
/// paints, or a visible effect other than Pathfinder (evaluated by [`pathfinder_children`])?
pub fn has_container_appearance(n: &Node) -> bool {
    is_container(n) && (has_own_paint(n) || n.appearance.effects.iter().any(|e| e.visible && !is_pathfinder(&e.id)))
}

/// The regions a group's or layer's own fills and strokes paint, in document space, each with its
/// fill rule: every visible member path and compound path, type as its glyph outlines and live
/// objects evaluated; the members of a clip group or clipping layer without its clipping path.
/// Images, symbol instances and guides add nothing.
pub fn member_shapes(n: &Node) -> Vec<(PathData, FillRule)> {
    fn push(n: &Node, out: &mut Vec<(PathData, FillRule)>) {
        if !n.visible {
            return;
        }
        match &n.kind {
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => out.extend(crate::reshape::leaf_geometry(n)),
            NodeKind::Text(t) => {
                let (_, all) = crate::reshape::glyph_outlines(t);
                if !all.elements().is_empty() {
                    out.push((PathData::from_bezpath(&all).transformed(t.xf), FillRule::NonZero));
                }
            }
            NodeKind::Group { .. } | NodeKind::Layer { .. } => members(n, out),
            _ if vectorcraft_doc::live::is_live(n) => members(&crate::reshape::outlined_members(n), out),
            _ => {}
        }
    }
    fn members(n: &Node, out: &mut Vec<(PathData, FillRule)>) {
        let clip = usize::from(n.clips());
        for c in n.children().into_iter().flatten().skip(clip) {
            push(c, out);
        }
    }
    let mut out = vec![];
    members(n, &mut out);
    out
}

/// A path node of `id` on `path` (filled under `rule`) painted by `ap`.
fn shape_node(id: NodeId, path: PathData, rule: FillRule, ap: Appearance) -> Node {
    let mut n = Node::path(id, path, ap);
    if let NodeKind::Path { rule: r, .. } = &mut n.kind {
        *r = rule;
    }
    n
}

/// One of a container's own fills or strokes as art: `item` painting every shape of `shapes`, an
/// unplaced gradient placed across `bounds` (the whole group). Several shapes share the item's
/// opacity and blend mode as one layer, as one fill would. `None` when it paints nothing.
fn item_art(id: NodeId, item: &AppearanceItem, shapes: &[(PathData, FillRule)], bounds: Option<Rect>) -> Option<Node> {
    if !paints(item) || shapes.is_empty() {
        return None;
    }
    let mut ap = Appearance { items: vec![item.clone()], ..Default::default() };
    if let Some(b) = bounds {
        ap.pin_gradients(b);
    }
    if let [(path, rule)] = shapes {
        return Some(shape_node(id, path.clone(), *rule, ap));
    }
    let (opacity, blend) = (item.opacity(), item.blend());
    match &mut ap.items[0] {
        AppearanceItem::Fill(f) => (f.opacity, f.blend) = (1.0, Default::default()),
        AppearanceItem::Stroke(s) => (s.opacity, s.blend) = (1.0, Default::default()),
    }
    let children = shapes.iter().map(|(p, r)| Arc::new(shape_node(id, p.clone(), *r, ap.clone()))).collect();
    let mut g = Node::group(id, children);
    (g.opacity, g.blend) = (opacity, blend);
    Some(g)
}

/// `n`, a group or layer, as plain art: its Pathfinder effects evaluated, its geometry effects
/// reshaping all its members as one piece (the whole art's bounds as their reference box), and its
/// own fills and strokes turned into art painting the members' geometry ([`member_shapes`]) below
/// the members (after a clipping path) or above them, as its contents slot says. Its
/// transparency, opacity mask and raster effects stay. `None` when it has none of these.
pub fn evaluate_container(n: &Node) -> Option<Node> {
    if !is_container(n) {
        return None;
    }
    let pathfinder = has_pathfinder(n);
    let fx = crate::reshape::geometry_effects(n);
    let paint = has_own_paint(n);
    if !pathfinder && fx.is_empty() && !paint {
        return None;
    }
    let mut m = n.clone();
    if pathfinder {
        let hook: &dyn Fn(&Node) -> Option<Node> = &crate::outline_text;
        let result = pathfinder_children(n, Some(hook)).unwrap_or_default();
        if let Some(ch) = m.children_mut() {
            *ch = result;
        }
    }
    if !fx.is_empty() {
        m = crate::reshape::outlined_members(&m);
        crate::reshape::reshape_leaves(&mut m, &fx);
    }
    m.appearance.effects.retain(|e| !is_pathfinder(&e.id) && !crate::is_geometry(&e.id));
    if paint {
        let shapes = member_shapes(&m);
        let bounds = m.geometric_bounds();
        let art =
            |items: &[AppearanceItem]| -> Vec<Arc<Node>> { items.iter().filter_map(|i| item_art(n.id, i, &shapes, bounds)).map(Arc::new).collect() };
        let (below, above) = m.appearance.split_contents();
        let (below, above) = (art(below), art(above));
        let at = usize::from(m.clips());
        if let Some(ch) = m.children_mut() {
            let at = at.min(ch.len());
            ch.splice(at..at, below);
            ch.extend(above);
        }
    }
    m.appearance.items.clear();
    m.appearance.contents_index = None;
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_color::{BlendMode, Color, Paint};
    use vectorcraft_doc::{CharStyle, Effect, NodeId, TextObject};
    use vectorcraft_geom::{Point, Rect, shapes};

    fn sq(id: u64, x: f64, color: Color) -> Arc<Node> {
        Arc::new(Node::path(
            NodeId(id),
            shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)),
            Appearance::basic(Paint::solid(color), Paint::None, 0.0),
        ))
    }

    fn group_with(effect: &str) -> Node {
        let mut g = Node::group(NodeId(10), vec![sq(1, 0.0, Color::rgb(1.0, 0.0, 0.0)), sq(2, 5.0, Color::rgb(0.0, 0.0, 1.0))]);
        g.appearance.effects.push(Effect { id: effect.into(), params: serde_json::json!({}), visible: true });
        g
    }

    fn area(n: &Node) -> f64 {
        po::area(n.path_data().unwrap(), FillRule::NonZero).abs()
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
        let art = evaluate_container(&g).unwrap();
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
        let art = evaluate_container(&g).unwrap();
        (*art.children().unwrap()[1]).clone()
    }

    #[test]
    fn type_in_a_group_keeps_its_look_when_the_groups_geometry_effects_outline_it() {
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
        use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
        let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
        let ramp = GradientPaint::new(Gradient { kind: GradientKind::Linear, stops });
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
        let art = evaluate_container(&g).unwrap();
        let members = art.children().unwrap();
        let after = start(&members[1].children().unwrap()[0]);
        assert!((after - before - 30.0).abs() < 1e-6, "the type's gradient starts at {before}, then {after}");
        assert_eq!(start(&members[0]), 0.0, "the square's gradient stays where it was placed");
    }
}
