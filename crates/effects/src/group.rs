//! Effect → Pathfinder: live Pathfinder operations on a group (or layer).
//!
//! The group's direct children are the Pathfinder stack (back → front). In the shape modes (Add,
//! Subtract, Intersect, Exclude, Minus Back) a child group acts as one shape — the union of its
//! leaves, styled like its top leaf — as in the Pathfinder panel. The result replaces the children
//! at render time; each result keeps the appearance of the object whose paint it takes.

use std::sync::Arc;

use vectorcraft_doc::{Appearance, AppearanceItem, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData};
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
    let mut appearance = Appearance { items: src.appearance.items.clone(), effects: vec![] };
    if outline {
        let fill = src.appearance.fill_paint();
        let paint = if fill.is_none() { src.appearance.stroke_paint() } else { fill };
        appearance = Appearance { items: vec![AppearanceItem::Stroke(StrokeLayer::new(paint, 1.0))], effects: vec![] };
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
                let united = po::unite_all(&refs);
                if let Some(last) = lv.pop() {
                    shapes.push(po::Shape::new(united, FillRule::NonZero, sources.len() as u64));
                    sources.push(last);
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
    fn hidden_effect_or_plain_group_is_untouched() {
        let mut g = group_with("pathfinder.add");
        g.appearance.effects[0].visible = false;
        assert!(pathfinder_children(&g, None).is_none());
        assert!(!has_pathfinder(&sq(1, 0.0, Color::BLACK)));
    }
}
