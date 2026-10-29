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
#[allow(unknown_lints, clippy::manual_isolate_lowest_one)]
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
mod tests;
