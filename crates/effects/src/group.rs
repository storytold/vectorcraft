//! The appearance of groups and layers.
//!
//! **Effect → Pathfinder**: live Pathfinder operations on a group (or layer). The group's direct
//! children are the Pathfinder stack (back → front). In the shape modes (Add, Subtract, Intersect,
//! Exclude, Minus Back) a child group acts as one shape — the union of its leaves, styled like its
//! top leaf — as in the Pathfinder panel. The result replaces the children at render time; each
//! result keeps the appearance of the object whose paint it takes. Trap ([`crate::trap`]) keeps the
//! children and adds its traps above them.
//!
//! **Compound shapes** ([`compound_shape_area`]): a compound shape draws as the one area its
//! members' shape modes make, painted by its own appearance as a group's own fills and strokes
//! paint its members.
//!
//! **Container appearance** ([`evaluate_container`]): a group's or layer's own fills and strokes
//! paint every member's geometry, below or above the members as its contents slot says
//! ([`Appearance::contents_at`]); its geometry effects reshape the members as one piece; its raster
//! effects (one combined shadow) apply to the composite and stay for the renderer and exporters.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Node, NodeId, NodeKind, ShapeMode, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData, Rect};
use vectorcraft_pathops::{self as po, PathfinderOp};

use crate::reshape::SymbolNodes;

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

/// Does `n` (a group or layer) carry a visible Pathfinder effect, or is it a compound shape?
pub fn has_pathfinder(n: &Node) -> bool {
    n.is_compound_shape()
        || (matches!(n.kind, NodeKind::Group { clip: false, .. } | NodeKind::Layer { .. })
            && n.appearance.effects.iter().any(|e| e.visible && is_pathfinder(&e.id)))
}

/// Converts objects that aren't paths (e.g. text) to outline art, if possible. The paths and
/// compound paths in that art, at every depth of its groups, take part in the Pathfinder operation
/// with the object's opacity and blend mode.
pub type OutlineHook<'a> = &'a dyn Fn(&Node) -> Option<Node>;

/// The paths and compound paths `n` takes part in a compound shape or a Pathfinder operation as,
/// back to front: a compound shape as its area, a blend, an envelope or an object with a warp as
/// the art it draws, and type and symbol instances as their art through `hook`. Past
/// [`SYMBOL_ART_MAX_NODES`](crate::SYMBOL_ART_MAX_NODES) objects of symbol art, further
/// instances add nothing.
pub fn member_leaves(n: &Node, hook: Option<OutlineHook>) -> Vec<Node> {
    let mut out = vec![];
    leaves(n, hook, &SymbolNodes::default(), &mut out);
    out
}

/// [`member_leaves`], with the symbol art given so far counted in `nodes`.
fn leaves(n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes, out: &mut Vec<Node>) {
    if !n.visible {
        return;
    }
    // A blend, an envelope or an object with a warp takes part as the art it draws.
    if let Some(art) = shape_art_in(n, hook, nodes) {
        let first = out.len();
        leaves(&art, hook, nodes, out);
        // Every path of a live object's, type's or symbol instance's art is drawn inside the
        // object's opacity and blend mode.
        if crate::needs_outline(n) {
            for l in out.iter_mut().skip(first) {
                within(l, n);
            }
        }
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => out.push(n.clone()),
        // A compound shape takes part as its area, styled like it.
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => match compound_shape_art_in(n, hook, nodes) {
            Some(art) => out.push(art),
            None => {
                for c in children {
                    leaves(c, hook, nodes, out);
                }
            }
        },
        _ => {
            // Every path of the outline art, at any depth, is drawn inside the object's opacity
            // and blend mode. A symbol's art comes with the live objects and the instances of
            // other symbols inside it given as their art, and the type inside it outlined, so the
            // art has nothing left for `hook` to give.
            if let Some(h) = hook
                && let Some(art) = h(n)
            {
                let art = match &n.kind {
                    NodeKind::SymbolInstance { symbol, .. } if nodes.charge(&art) => {
                        let mut art = crate::reshape::symbol_art_with(&art, h, nodes, symbol);
                        crate::reshape::outline_nested_text(&mut art, false);
                        transparency_into_members(&mut art);
                        art
                    }
                    NodeKind::SymbolInstance { .. } => return,
                    _ => art,
                };
                let first = out.len();
                leaves(&art, None, nodes, out);
                for l in out.iter_mut().skip(first) {
                    within(l, n);
                }
            }
        }
    }
}

/// `l`, a path drawn inside `n`: its opacity multiplied by `n`'s, and `n`'s blend mode when it is
/// not Normal.
fn within(l: &mut Node, n: &Node) {
    drawn_in(l, n.opacity, n.blend);
}

/// `l` drawn inside a group of `opacity` and `blend`: its opacity multiplied by `opacity`, and
/// `blend` when it is not Normal.
fn drawn_in(l: &mut Node, opacity: f32, blend: BlendMode) {
    l.opacity *= opacity;
    if blend != BlendMode::Normal {
        l.blend = blend;
    }
}

/// Move the opacity and blend mode of every group and layer inside `n` (not `n` itself, and not
/// compound shapes, which take part with their own) onto the objects it holds ([`drawn_in`]), so
/// that each path of `n` carries the transparency it is drawn with.
fn transparency_into_members(n: &mut Node) {
    let Some(ch) = n.children_mut() else { return };
    for c in ch.iter_mut() {
        if !is_container(c) || c.is_compound_shape() {
            continue;
        }
        let c = Arc::make_mut(c);
        let (opacity, blend) = (c.opacity, c.blend);
        if let Some(inner) = c.children_mut() {
            for g in inner.iter_mut() {
                drawn_in(Arc::make_mut(g), opacity, blend);
            }
        }
        (c.opacity, c.blend) = (1.0, BlendMode::Normal);
        transparency_into_members(c);
    }
}

/// `n` as the art its visible geometry gives a compound shape or a Pathfinder operation, where that
/// is not its own paths: a live object evaluated (a blend's steps, an envelope's distorted art,
/// type inside it outlined), and a path, compound path, group, compound shape, live object, type or
/// symbol instance with visible geometry effects (such as a warp) reshaped by them, as the renderer
/// draws it. Type and symbol instances inside the art are given as their art by `hook` (without
/// one, type is outlined and symbol instances stay as they are);
/// [`outline_object`](crate::outline_object) gives a symbol instance its symbol's art. The art
/// keeps the effects of `n` other than its geometry effects, and the groups inside it pass their
/// opacity and blend mode to the objects they hold. `None` when `n` is none of these, for images,
/// for symbol instances without geometry effects, and for one without art from `hook`.
pub fn shape_art(n: &Node, hook: Option<OutlineHook>) -> Option<Node> {
    shape_art_in(n, hook, &SymbolNodes::default())
}

/// [`shape_art`], with the symbol art given so far counted in `nodes`.
fn shape_art_in(n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes) -> Option<Node> {
    let mut art = reshaped_art(n, hook, nodes)?;
    transparency_into_members(&mut art);
    Some(art)
}

/// [`shape_art_in`] before the groups inside the art pass on their transparency.
fn reshaped_art(n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes) -> Option<Node> {
    let fx = crate::reshape::geometry_effects(n);
    let outline = hook.unwrap_or(&crate::outline_text);
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } if !fx.is_empty() => Some(reshaped(n, n)),
        NodeKind::Group { .. } | NodeKind::Layer { .. } if !fx.is_empty() => match compound_shape_art_in(n, hook, nodes) {
            Some(area) => Some(reshaped(&area, n)),
            None => {
                let mut m = crate::reshape::outlined_members_with(None, n, outline, nodes);
                crate::reshape::reshape_leaves(&mut m, &fx);
                without_geometry_effects(&mut m);
                Some(m)
            }
        },
        // A symbol instance with a warp: its symbol's art placed by the instance, with the type
        // inside it outlined, reshaped as one piece, as the renderer draws it.
        NodeKind::SymbolInstance { symbol, .. } if !fx.is_empty() => {
            let h = hook?;
            let art = h(n).filter(|art| nodes.charge(art))?;
            let mut art = crate::reshape::symbol_art_with(&art, h, nodes, symbol);
            crate::reshape::outline_nested_text(&mut art, false);
            crate::reshape::reshape_leaves(&mut art, &fx);
            (art.opacity, art.blend, art.isolate) = (n.opacity, n.blend, n.isolate);
            art.appearance.effects = n.appearance.effects.clone();
            without_geometry_effects(&mut art);
            Some(art)
        }
        NodeKind::Image(_) | NodeKind::SymbolInstance { .. } => None,
        _ if crate::needs_outline(n) && (vectorcraft_doc::live::is_live(n) || !fx.is_empty()) => {
            crate::reshape::as_art_with(None, n, None, outline, nodes)
        }
        _ => None,
    }
}

/// `src`, a path, compound path or compound shape's area, as one path reshaped by the visible
/// geometry effects of `n`, without those effects.
fn reshaped(src: &Node, n: &Node) -> Node {
    let mut m = src.clone();
    if let Some((path, rule)) = geometry(src) {
        let bounds = vectorcraft_geom::Shape::bounding_box(&path.to_bezpath());
        let g = crate::apply_geometry_with(&n.appearance.effects, &path, bounds, &crate::GeomContext::of(n));
        m.kind = NodeKind::Path { path: g, rule, live: None, clipping: false, guide: false };
    }
    without_geometry_effects(&mut m);
    m
}

fn without_geometry_effects(n: &mut Node) {
    if n.appearance.effects.iter().any(|e| crate::is_geometry(&e.id)) {
        let mut ap = (*n.appearance).clone();
        ap.effects.retain(|e| !crate::is_geometry(&e.id));
        n.appearance = ap.into();
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

/// The operation that combines a compound shape's member in `mode` with the area below it.
fn shape_op(mode: ShapeMode) -> po::BoolOp {
    match mode {
        ShapeMode::Add => po::BoolOp::Union,
        ShapeMode::Subtract => po::BoolOp::Difference,
        ShapeMode::Intersect => po::BoolOp::Intersect,
        ShapeMode::Exclude => po::BoolOp::Xor,
    }
}

/// The area compound shape `n` draws: each visible member's shape (a group's leaves together, type
/// as its outlines through `hook`, open paths closed) combined with the area of the members below
/// it by the member's shape mode; the backmost member starts the area. `None` when `n` is not a
/// compound shape, or its members are past a live Pathfinder's budget ([`PATHFINDER_MAX_SEGMENTS`],
/// [`PATHFINDER_MAX_OVERLAPS`], [`SYMBOL_ART_MAX_NODES`](crate::SYMBOL_ART_MAX_NODES)).
pub fn compound_shape_area(n: &Node, hook: Option<OutlineHook>) -> Option<PathData> {
    compound_shape_area_in(n, hook, &SymbolNodes::default())
}

/// [`compound_shape_area`], with the symbol art given so far counted in `nodes`.
fn compound_shape_area_in(n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes) -> Option<PathData> {
    if !n.is_compound_shape() {
        return None;
    }
    let mut all: Vec<Node> = vec![];
    // Each member that has a shape: its operation and how many of the leaves are its.
    let mut members: Vec<(po::BoolOp, usize)> = vec![];
    for c in n.children()? {
        let before = all.len();
        leaves(c, hook, nodes, &mut all);
        if all.len() > before {
            members.push((shape_op(c.extras().shape_mode), all.len() - before));
        }
    }
    if nodes.spent() || !within_budget(&all) {
        return None;
    }
    if all.is_empty() {
        return Some(PathData::default());
    }
    let parts: Vec<(PathData, FillRule)> = all.iter().map(|l| geometry(l).unwrap_or_default()).collect();
    let refs: Vec<(&PathData, FillRule)> = parts.iter().map(|(p, r)| (p, *r)).collect();
    Some(po::boolean_n(&refs, |inside| {
        let (mut start, mut area) = (0, false);
        for (k, (op, count)) in members.iter().enumerate() {
            // `inside` is ascending: the first leaf at or after `start` tells whether one of this member's is in it.
            let here = inside.get(inside.partition_point(|&i| i < start)).is_some_and(|&i| i < start + count);
            start += count;
            area = if k == 0 { here } else { op.apply(area, here) };
        }
        area
    }))
}

/// Compound shape `n` as one path of its area ([`compound_shape_area`]) with its appearance,
/// opacity and blend mode: how it takes part in a Pathfinder operation.
pub fn compound_shape_art(n: &Node, hook: Option<OutlineHook>) -> Option<Node> {
    compound_shape_art_in(n, hook, &SymbolNodes::default())
}

/// [`compound_shape_art`], with the symbol art given so far counted in `nodes`.
fn compound_shape_art_in(n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes) -> Option<Node> {
    let area = compound_shape_area_in(n, hook, nodes)?;
    let mut art = Node::path(n.id, area, Appearance { items: n.appearance.items.clone(), ..Default::default() });
    (art.opacity, art.blend) = (n.opacity, n.blend);
    Some(art)
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
/// [`PATHFINDER_MAX_OVERLAPS`], [`SYMBOL_ART_MAX_NODES`](crate::SYMBOL_ART_MAX_NODES) for
/// each effect) it returns the children as they are.
pub fn pathfinder_children(group: &Node, hook: Option<OutlineHook>) -> Option<Vec<Arc<Node>>> {
    if !has_pathfinder(group) {
        return None;
    }
    let source: &[Arc<Node>] = group.children()?;
    let mut stack: Vec<Arc<Node>> = if group.is_compound_shape() {
        // The group's own appearance paints the area ([`evaluate_container`]), so the art has none.
        match compound_shape_area(group, hook) {
            Some(area) if area.is_empty() => vec![],
            Some(area) => vec![Arc::new(Node::path(group.id, area, Appearance::default()))],
            None => return Some(source.to_vec()),
        }
    } else {
        source.to_vec()
    };
    for e in group.appearance.effects.iter().filter(|e| e.visible) {
        if e.id == crate::trap::TRAP {
            let (mut lv, nodes) = (vec![], SymbolNodes::default());
            for c in &stack {
                leaves(c, hook, &nodes, &mut lv);
            }
            if nodes.spent() || !within_budget(&lv) {
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
        let nodes = SymbolNodes::default();
        let all: Vec<Vec<Node>> = stack
            .iter()
            .map(|c| {
                let mut lv = vec![];
                leaves(c, hook, &nodes, &mut lv);
                lv
            })
            .collect();
        if nodes.spent() || !within_budget(all.iter().flatten()) {
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
/// objects evaluated; a compound shape as its area, with the symbol instances among its members
/// as their symbols' art from `doc`; the members of a clip group or clipping layer without its
/// clipping path. Images, other symbol instances and guides add nothing. A Repeat counts the
/// Repeats in the art of `doc`'s symbols that its instances show toward its instance budget.
pub fn member_shapes(doc: Option<&Document>, n: &Node) -> Vec<(PathData, FillRule)> {
    fn push(doc: Option<&Document>, n: &Node, out: &mut Vec<(PathData, FillRule)>) {
        if !n.visible {
            return;
        }
        match &n.kind {
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => out.extend(crate::reshape::leaf_geometry(n)),
            NodeKind::Group { .. } if n.is_compound_shape() => {
                let hook = |c: &Node| crate::outline_object(doc, c);
                match compound_shape_area(n, Some(&hook)) {
                    Some(area) => out.push((area, FillRule::NonZero)),
                    None => members(doc, n, out),
                }
            }
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
/// objects among the members are evaluated with the symbols of `doc` (see [`member_shapes`]), and
/// a Pathfinder operation or compound shape takes the symbol instances among its members as their
/// symbols' art from `doc`.
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
        let hook = |c: &Node| crate::outline_object(doc, c);
        let result = pathfinder_children(n, Some(&hook)).unwrap_or_default();
        if let Some(ch) = m.children_mut() {
            *ch = result;
        }
        // The area is art now: drawn again, it is no compound shape of its own.
        m.extras_mut().compound_shape = false;
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
