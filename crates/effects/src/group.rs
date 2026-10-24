//! The appearance of groups and layers.
//!
//! **Effect → Pathfinder**: live Pathfinder operations on a group (or layer). The group's direct
//! children are the Pathfinder stack (back → front). In the shape modes (Add, Subtract, Intersect,
//! Exclude, Minus Back) a child group acts as one shape — the union of its leaves, styled like its
//! top leaf — as in the Pathfinder panel. The result replaces the children at render time; each
//! result keeps the appearance of the object whose paint it takes. Trap ([`crate::trap`]) keeps the
//! children and adds its traps above them.
//!
//! **Container appearance** ([`evaluate_container`]): a group's or layer's own fills and strokes
//! paint every member's geometry, below or above the members as its contents slot says
//! ([`Appearance::contents_at`]); its geometry effects reshape the members as one piece; its raster
//! effects (one combined shadow) apply to the composite and stay for the renderer and exporters.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData, Rect};
use vectorcraft_pathops::{self as po, PathfinderOp};

/// Effect ids of the Pathfinder submenu with their operation, in Illustrator's menu order.
pub const PATHFINDER_EFFECTS: [(&str, &str, PathfinderOp); 12] = [
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
    ("pathfinder.hardMix", "Hard Mix", PathfinderOp::HardMix),
    ("pathfinder.softMix", "Soft Mix", PathfinderOp::SoftMix),
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

/// Most segments a live Pathfinder effect takes in, as for the Shape Builder.
pub const PATHFINDER_MAX_SEGMENTS: usize = po::SHAPE_BUILDER_MAX_SEGMENTS;

/// Most pairs of input paths whose bounds overlap a live Pathfinder effect takes in. Crossings,
/// and so the pieces of the result, grow with these pairs (n bars crossing n make n² crossings)
/// and the effect runs on every draw: past this the group draws its art unprocessed (#3591).
pub const PATHFINDER_MAX_OVERLAPS: usize = 16_384;

/// Fenwick tree of counts over `n` slots, for counting the active boxes of [`within_budget`].
struct Counts(Vec<isize>);

// `isolate_lowest_one` is still unstable in Rust 1.96, and the workspace builds on 1.95 (rust-version).
#[allow(clippy::manual_isolate_lowest_one)]
impl Counts {
    fn add(&mut self, slot: usize, d: isize) {
        let mut i = slot + 1;
        while let Some(c) = self.0.get_mut(i) {
            *c += d;
            i += i & i.wrapping_neg();
        }
    }

    /// Sum of the slots before `end`.
    fn before(&self, end: usize) -> isize {
        let (mut i, mut s) = (end.min(self.0.len().saturating_sub(1)), 0);
        while i > 0 {
            s += self.0.get(i).copied().unwrap_or(0);
            i -= i & i.wrapping_neg();
        }
        s
    }
}

/// Can a live Pathfinder evaluate `leaves` within [`PATHFINDER_MAX_SEGMENTS`] and
/// [`PATHFINDER_MAX_OVERLAPS`]? The overlapping pairs of bounds are counted exactly by a sweep
/// in x that keeps the active boxes in two Fenwick trees over their y extents, so the check costs
/// O(n log n) however the boxes are laid out.
fn within_budget<'a>(leaves: impl IntoIterator<Item = &'a Node>) -> bool {
    let mut segments = 0usize;
    let mut boxes: Vec<Rect> = vec![];
    for l in leaves {
        let Some((path, _)) = geometry(l) else { continue };
        segments += path.subpaths.iter().map(|sp| sp.segment_count()).sum::<usize>();
        if segments > PATHFINDER_MAX_SEGMENTS {
            return false;
        }
        boxes.extend(path.bounds());
    }
    let mut ys: Vec<f64> = boxes.iter().flat_map(|b| [b.y0, b.y1]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| a.total_cmp(b).is_eq());
    // Slots of the y values below `y`, and of those at or below it.
    let below = |y: f64| ys.partition_point(|v| v.total_cmp(&y).is_lt());
    let upto = |y: f64| ys.partition_point(|v| v.total_cmp(&y).is_le());
    // (x, is end, box); starts sort before ends at the same x since touching boxes overlap.
    let mut events: Vec<(f64, bool, usize)> = boxes.iter().enumerate().flat_map(|(i, b)| [(b.x0, false, i), (b.x1, true, i)]).collect();
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    // Active boxes counted by the slot of their y0 and of their y1.
    let mut by_y0 = Counts(vec![0; ys.len() + 1]);
    let mut by_y1 = Counts(vec![0; ys.len() + 1]);
    let (mut active, mut overlaps) = (0isize, 0usize);
    for (_, end, i) in events {
        let Some(b) = boxes.get(i) else { continue };
        if end {
            by_y0.add(below(b.y0), -1);
            by_y1.add(below(b.y1), -1);
            active -= 1;
            continue;
        }
        // Active boxes not entirely below b (y1 < b.y0) nor entirely above it (y0 > b.y1).
        let above = active - by_y0.before(upto(b.y1));
        let under = by_y1.before(below(b.y0));
        overlaps += usize::try_from(active - above - under).unwrap_or(0);
        if overlaps > PATHFINDER_MAX_OVERLAPS {
            return false;
        }
        by_y0.add(below(b.y0), 1);
        by_y1.add(below(b.y1), 1);
        active += 1;
    }
    true
}

/// Evaluate the visible Pathfinder effects of `group` in order: the resulting child objects.
/// `None` when the group has no Pathfinder effect. Past the budget ([`PATHFINDER_MAX_SEGMENTS`],
/// [`PATHFINDER_MAX_OVERLAPS`]) it returns the children as they are.
pub fn pathfinder_children(group: &Node, hook: Option<OutlineHook>) -> Option<Vec<Arc<Node>>> {
    if !has_pathfinder(group) {
        return None;
    }
    let source: &[Arc<Node>] = group.children()?;
    let mut stack: Vec<Arc<Node>> = source.to_vec();
    for e in group.appearance.effects.iter().filter(|e| e.visible) {
        if e.id == crate::trap::TRAP {
            let mut lv = vec![];
            for c in &stack {
                leaves(c, hook, &mut lv);
            }
            if !within_budget(&lv) {
                return Some(source.to_vec());
            }
            let (ids, srcs): (Vec<NodeId>, Vec<crate::trap::TrapSource>) = lv
                .iter()
                .filter_map(|l| {
                    let (path, rule) = geometry(l)?;
                    Some((l.id, crate::trap::TrapSource { path, rule, fill: l.appearance.fill_paint() }))
                })
                .unzip();
            let made = crate::trap::traps(&srcs, &crate::trap::TrapOptions::from_params(&e.params));
            stack.extend(made.into_iter().map(|p| Arc::new(crate::trap::trap_node(ids.get(p.from).copied().unwrap_or(NodeId(0)), p))));
            continue;
        }
        let Some(op) = op_of(&e.id) else { continue };
        let shape_mode =
            matches!(op, PathfinderOp::Unite | PathfinderOp::MinusFront | PathfinderOp::Intersect | PathfinderOp::Exclude | PathfinderOp::MinusBack);
        let all: Vec<Vec<Node>> = stack
            .iter()
            .map(|c| {
                let mut lv = vec![];
                leaves(c, hook, &mut lv);
                lv
            })
            .collect();
        if !within_budget(all.iter().flatten()) {
            return Some(source.to_vec());
        }
        // (style source, shape) per stack entry; keys index `sources`.
        let mut sources: Vec<Node> = vec![];
        let mut shapes: Vec<po::Shape> = vec![];
        for mut lv in all {
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
        stack = if op == PathfinderOp::SoftMix {
            let rate = mixing_rate(&e.params);
            po::regions(&shapes)
                .into_iter()
                .filter(|r| !r.path.is_empty())
                .map(|r| Arc::new(soft_mix_face(r.path, &r.sources, rate, &shapes, &sources)))
                .collect()
        } else if op == PathfinderOp::HardMix {
            po::regions(&shapes)
                .into_iter()
                .filter(|r| !r.path.is_empty())
                .map(|r| Arc::new(hard_mix_face(r.path, &r.sources, &shapes, &sources)))
                .collect()
        } else {
            po::pathfinder(op, &shapes)
                .into_iter()
                .filter(|r| !r.path.is_empty())
                .map(|r| Arc::new(styled(&sources[r.key as usize], r.path, op == PathfinderOp::Outline)))
                .collect()
        };
    }
    Some(stack)
}

fn mixing_rate(params: &Value) -> f64 {
    params.get("mixingRate").and_then(|v| v.as_f64()).filter(|r| r.is_finite() && *r > 0.0 && *r <= 100.0).map(|r| r / 100.0).unwrap_or(0.5)
}

fn fill_cmyk(src: &Node) -> Option<[f32; 4]> {
    let paint = src.appearance.fill_paint();
    let color = paint.color()?;
    let tinted = match paint {
        Paint::Solid { tint, .. } => color.tinted(tint),
        _ => color,
    };
    Some(tinted.to_cmyk())
}

/// One face of a Soft Mix: CMYK blended bottom to top at `rate`; no stroke.
fn soft_mix_face(path: PathData, covering: &[usize], rate: f64, shapes: &[po::Shape], sources: &[Node]) -> Node {
    let rate = rate as f32;
    let mut cmyk: Option<[f32; 4]> = None;
    for &si in covering {
        let Some(ch) = fill_cmyk(&sources[shapes[si].key as usize]) else { continue };
        cmyk = Some(match cmyk {
            None => ch,
            Some(acc) => {
                let t = rate;
                let u = 1.0 - t;
                [acc[0] * u + ch[0] * t, acc[1] * u + ch[1] * t, acc[2] * u + ch[2] * t, acc[3] * u + ch[3] * t]
            }
        });
    }
    let fill = cmyk.map(|c| Paint::solid(Color::cmyk(c[0], c[1], c[2], c[3]))).unwrap_or(Paint::None);
    soft_mix_styled(path, covering, shapes, sources, fill)
}

/// One face of a Hard Mix: the max of each CMYK channel over every covering shape's fill; no stroke.
fn hard_mix_face(path: PathData, covering: &[usize], shapes: &[po::Shape], sources: &[Node]) -> Node {
    let mut cmyk = [0.0f32; 4];
    let mut painted = false;
    for &si in covering {
        let Some(ch) = fill_cmyk(&sources[shapes[si].key as usize]) else { continue };
        for (out, v) in cmyk.iter_mut().zip(ch) {
            *out = out.max(v);
        }
        painted = true;
    }
    let fill = if painted { Paint::solid(Color::cmyk(cmyk[0], cmyk[1], cmyk[2], cmyk[3])) } else { Paint::None };
    soft_mix_styled(path, covering, shapes, sources, fill)
}

fn soft_mix_styled(path: PathData, covering: &[usize], shapes: &[po::Shape], sources: &[Node], fill: Paint) -> Node {
    let top = covering.last().and_then(|&si| sources.get(shapes[si].key as usize));
    let mut n = Node::path(top.map_or(NodeId(0), |s| s.id), path, Appearance::basic(fill, Paint::None, 0.0));
    if let Some(src) = top {
        n.opacity = src.opacity;
        n.blend = src.blend;
    }
    n
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
/// Images, symbol instances and guides add nothing. A Repeat counts the Repeats in the art of
/// `doc`'s symbols that its instances show toward its instance budget.
pub fn member_shapes(doc: Option<&Document>, n: &Node) -> Vec<(PathData, FillRule)> {
    fn push(doc: Option<&Document>, n: &Node, out: &mut Vec<(PathData, FillRule)>) {
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
            NodeKind::Group { .. } | NodeKind::Layer { .. } => members(doc, n, out),
            _ if vectorcraft_doc::live::is_live(n) => members(doc, &crate::reshape::outlined_members(doc, n), out),
            _ => {}
        }
    }
    fn members(doc: Option<&Document>, n: &Node, out: &mut Vec<(PathData, FillRule)>) {
        let clip = usize::from(n.clips());
        for c in n.children().into_iter().flatten().skip(clip) {
            push(doc, c, out);
        }
    }
    let mut out = vec![];
    members(doc, n, &mut out);
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
/// transparency, opacity mask and raster effects stay. `None` when it has none of these. Live
/// objects among the members are evaluated with the symbols of `doc` (see [`member_shapes`]).
pub fn evaluate_container(doc: Option<&Document>, n: &Node) -> Option<Node> {
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
        m = crate::reshape::outlined_members(doc, &m);
        crate::reshape::reshape_leaves(&mut m, &fx);
    }
    m.appearance.effects.retain(|e| !is_pathfinder(&e.id) && !crate::is_geometry(&e.id));
    if paint {
        let shapes = member_shapes(doc, &m);
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

    /// Type tests need glyph outlines, which only the optional craft-fonts build input provides.
    fn skip_without_craft_fonts() -> bool {
        let skip = vectorcraft_text::CRAFT_FONTS.is_empty();
        if skip {
            eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        }
        skip
    }

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
            let square =
                Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(paint, Paint::None, 0.0)));
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
}
