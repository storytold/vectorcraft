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

/// Converts objects that aren't paths (e.g. text) to an outline path node, if possible.
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
            if let Some(p) = hook.and_then(|h| h(n)) {
                out.push(p);
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
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Effect, NodeId};
    use vectorcraft_geom::{Rect, shapes};

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

    #[test]
    fn hidden_effect_or_plain_group_is_untouched() {
        let mut g = group_with("pathfinder.add");
        g.appearance.effects[0].visible = false;
        assert!(pathfinder_children(&g, None).is_none());
        assert!(!has_pathfinder(&sq(1, 0.0, Color::BLACK)));
    }
}
