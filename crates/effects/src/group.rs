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

/// Does effect `id` keep Pathfinder Options of its own ([`PathfinderEffectOptions`]): one of
/// [`PATHFINDER_EFFECTS`] (Trap has options of its own)?
pub fn takes_pathfinder_options(id: &str) -> bool {
    op_of(id).is_some()
}

/// The parameters holding a Pathfinder effect's Pathfinder Options.
pub const PATHFINDER_OPTION_KEYS: [&str; 3] = ["precision", "removeRedundantPoints", "removeUnpainted"];

/// The range of a Pathfinder effect's `precision` (pt), the Pathfinder panel's range.
pub const PATHFINDER_EFFECT_PRECISION: std::ops::RangeInclusive<f64> = 0.001..=10.0;

/// The documentation of [`PATHFINDER_OPTION_KEYS`], as a literal for `concat!`.
macro_rules! options_doc {
    () => {
        "precision?: pt 0.001..10 (effect.apply: Pathfinder Options › Precision; 0.01 when missing; results are refit to within it and pieces thinner than it are dropped), removeRedundantPoints?: bool (effect.apply: Pathfinder Options › Remove Redundant Points; false when missing; results lose the anchors that don't change their shape to within precision), removeUnpainted?: bool (effect.apply: Pathfinder Options › Divide and Outline Will Remove Unpainted Artwork; false when missing; Divide and Outline leave out the pieces of members with no visible fill, a compound shape's area going by the shape's own fill, except that Divide keeps every piece when the group or compound shape has a visible fill of its own; the other operations ignore it)"
    };
}

/// The documentation of [`PATHFINDER_OPTION_KEYS`], shared by the Pathfinder effects' parameters.
pub const PATHFINDER_OPTIONS_DOC: &str = options_doc!();

/// The parameters of a Pathfinder effect other than Soft Mix.
pub const PATHFINDER_PARAMS_DOC: &str = concat!("{", options_doc!(), "} (groups and layers: live Pathfinder over the members)");

/// The parameters of Effect › Pathfinder › Soft Mix.
pub const SOFT_MIX_PARAMS_DOC: &str = concat!(
    "{mixingRate?: 1..100 % (50), ",
    options_doc!(),
    "} blend overlapping fills in CMYK bottom to top (groups and layers: live Pathfinder over the members)"
);

/// The Pathfinder Options a Pathfinder effect evaluates with, read from its parameters. An effect
/// without them (saved before effects kept their own) evaluates as it did then: the default
/// precision, with no points or pieces removed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathfinderEffectOptions {
    /// Refit tolerance of the results, in points ([`PATHFINDER_EFFECT_PRECISION`]).
    pub precision: f64,
    /// Drop anchors that don't change a result's shape.
    pub remove_redundant_points: bool,
    /// Divide and Outline leave out the pieces of members with no visible fill.
    pub remove_unpainted: bool,
}

impl Default for PathfinderEffectOptions {
    fn default() -> Self {
        PathfinderEffectOptions { precision: po::DEFAULT_PRECISION, remove_redundant_points: false, remove_unpainted: false }
    }
}

impl PathfinderEffectOptions {
    /// The options in `p`: a missing or non-numeric value takes its default and a precision out of
    /// range its nearest bound, so a live effect always evaluates.
    pub fn from_params(p: &Value) -> Self {
        let d = PathfinderEffectOptions::default();
        let (lo, hi) = (*PATHFINDER_EFFECT_PRECISION.start(), *PATHFINDER_EFFECT_PRECISION.end());
        PathfinderEffectOptions {
            precision: crate::util::num(p, "precision", d.precision).clamp(lo, hi),
            remove_redundant_points: crate::util::flag(p, "removeRedundantPoints", d.remove_redundant_points),
            remove_unpainted: crate::util::flag(p, "removeUnpainted", d.remove_unpainted),
        }
    }

    /// These options as the parameters [`PATHFINDER_OPTION_KEYS`] name.
    pub fn to_params(self) -> Value {
        serde_json::json!({
            "precision": self.precision,
            "removeRedundantPoints": self.remove_redundant_points,
            "removeUnpainted": self.remove_unpainted,
        })
    }
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
/// instances add nothing. The swatches and patterns that live-effect plug-ins painting their
/// pieces name are looked up in `doc` ([`shape_art`]).
pub fn member_leaves(doc: Option<&Document>, n: &Node, hook: Option<OutlineHook>) -> Vec<Node> {
    let mut out = vec![];
    leaves(doc, n, hook, &SymbolNodes::default(), &mut out);
    out
}

/// [`member_leaves`], with the symbol art given so far counted in `nodes`.
fn leaves(doc: Option<&Document>, n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes, out: &mut Vec<Node>) {
    if !n.visible {
        return;
    }
    // A blend, an envelope or an object with a warp takes part as the art it draws.
    if let Some(art) = shape_art_in(doc, n, hook, nodes) {
        let first = out.len();
        leaves(doc, &art, hook, nodes, out);
        // Every path of a live object's, type's or symbol instance's art, and every piece of a path
        // whose live-effect plug-in paints its pieces, is drawn inside the object's opacity and
        // blend mode.
        let pieces = matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) && matches!(art.kind, NodeKind::Group { .. });
        if crate::needs_outline(n) || pieces {
            for l in out.iter_mut().skip(first) {
                within(l, n);
            }
        }
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => out.push(n.clone()),
        // A compound shape takes part as its area, styled like it.
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => match compound_shape_art_in(doc, n, hook, nodes) {
            Some(art) => out.push(art),
            None => {
                for c in children {
                    leaves(doc, c, hook, nodes, out);
                }
            }
        },
        _ => {
            // Every path of the outline art, at any depth, is drawn inside the object's opacity
            // and blend mode. A symbol's art comes with the live objects and the instances of
            // other symbols inside it given as their art, and the type inside it outlined, so the
            // art has nothing left for `hook` to give. Its fills and strokes carry the geometry
            // effects of the instance's active fill and stroke, as the canvas draws them.
            if let Some(h) = hook
                && let Some(art) = h(n)
            {
                let art = match &n.kind {
                    NodeKind::SymbolInstance { symbol, .. } if nodes.charge(&art) => {
                        let mut art = instance_art(doc, &art, h, nodes, symbol);
                        crate::reshape::add_instance_row_effects(doc, n, &mut art);
                        crate::reshape::outline_nested_text(doc, &mut art, false);
                        transparency_into_members(&mut art);
                        art
                    }
                    NodeKind::SymbolInstance { .. } => return,
                    _ => art,
                };
                let first = out.len();
                leaves(doc, &art, None, nodes, out);
                for l in out.iter_mut().skip(first) {
                    within(l, n);
                }
            }
        }
    }
}

/// `art`, the art `hook` gave an instance of the symbol named `symbol`, with the live objects and
/// the instances of other symbols inside it given as their art
/// ([`crate::reshape::symbol_art_with`]). The art of each of those instances carries the geometry
/// effects of its own active fill and stroke ([`crate::reshape::add_instance_row_effects`]).
fn instance_art(doc: Option<&Document>, art: &Node, hook: OutlineHook, nodes: &SymbolNodes, symbol: &str) -> Node {
    let drawn = |m: &Node| {
        let mut a = hook(m)?;
        crate::reshape::add_instance_row_effects(doc, m, &mut a);
        Some(a)
    };
    crate::reshape::symbol_art_with(art, &drawn, nodes, symbol)
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
/// for symbol instances without geometry effects, and for one without art from `hook`. A
/// live-effect plug-in that paints its pieces makes a path, and each path of a group's or symbol's
/// art, a group of its pieces, whose swatches and patterns must be in `doc` (without one, pieces
/// that name one are refused and the path stays as it is).
pub fn shape_art(doc: Option<&Document>, n: &Node, hook: Option<OutlineHook>) -> Option<Node> {
    shape_art_in(doc, n, hook, &SymbolNodes::default())
}

/// [`shape_art`], with the symbol art given so far counted in `nodes`.
fn shape_art_in(doc: Option<&Document>, n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes) -> Option<Node> {
    let mut art = reshaped_art(doc, n, hook, nodes)?;
    transparency_into_members(&mut art);
    Some(art)
}

/// [`shape_art_in`] before the groups inside the art pass on their transparency.
fn reshaped_art(doc: Option<&Document>, n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes) -> Option<Node> {
    let fx = crate::reshape::geometry_effects(n);
    let outline = hook.unwrap_or(&crate::outline_text);
    match &n.kind {
        // A path with a live-effect plug-in that paints its pieces: the group of its pieces, each
        // with its own paints, as the renderer draws it.
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } if !fx.is_empty() => {
            let painted = crate::reshape::has_painted_effects(n).then(|| crate::reshape::reshape(doc, n, None)).flatten();
            Some(painted.unwrap_or_else(|| reshaped(n, n, doc)))
        }
        NodeKind::Group { .. } | NodeKind::Layer { .. } if !fx.is_empty() => match compound_shape_art_in(doc, n, hook, nodes) {
            Some(area) => Some(reshaped(&area, n, doc)),
            None => {
                let mut m = crate::reshape::outlined_members_with(None, doc, n, outline, nodes);
                // The members' own effects run first, as on the canvas.
                members_as_drawn(doc, &mut m);
                let carry = crate::reshape::reshape_leaves_carrying(&mut m, &fx, &n.appearance.effects, doc);
                // The group's own fills and strokes follow the effects as its members do.
                if carry.moves(&m.appearance) {
                    carry.apply(&mut m.appearance);
                }
                without_geometry_effects(&mut m);
                Some(m)
            }
        },
        // A symbol instance with a warp: its symbol's art placed by the instance, with the type
        // inside it outlined, reshaped as one piece, as the renderer draws it. The fills and strokes
        // of the art then carry the geometry effects of the instance's active fill and stroke.
        NodeKind::SymbolInstance { symbol, .. } if !fx.is_empty() => {
            let h = hook?;
            let art = h(n).filter(|art| nodes.charge(art))?;
            let mut art = instance_art(doc, &art, h, nodes, symbol);
            crate::reshape::outline_nested_text(doc, &mut art, false);
            members_as_drawn(doc, &mut art);
            crate::reshape::add_instance_row_effects(doc, n, &mut art);
            let carry = crate::reshape::reshape_leaves_carrying(&mut art, &fx, &n.appearance.effects, doc);
            // The art's own fills and strokes follow the effects as its members do.
            if carry.moves(&art.appearance) {
                carry.apply(&mut art.appearance);
            }
            (art.opacity, art.blend, art.isolate) = (n.opacity, n.blend, n.isolate);
            art.appearance.effects = n.appearance.effects.clone();
            without_geometry_effects(&mut art);
            Some(art)
        }
        NodeKind::Image(_) | NodeKind::SymbolInstance { .. } => None,
        _ if crate::needs_outline(n) && (vectorcraft_doc::live::is_live(n) || !fx.is_empty()) => {
            crate::reshape::as_art_with(None, doc, n, None, outline, nodes)
        }
        _ => None,
    }
}

/// `src`, a path, compound path or compound shape's area, as one path reshaped by the visible
/// geometry effects of `n`, without those effects. The swatches and patterns a live-effect plug-in
/// that paints its pieces names are looked up in `doc`.
fn reshaped(src: &Node, n: &Node, doc: Option<&Document>) -> Node {
    let mut m = src.clone();
    if let Some((path, rule)) = geometry(src) {
        let bounds = vectorcraft_geom::Shape::bounding_box(&path.to_bezpath());
        let (g, carry) =
            crate::apply_geometry_carrying(&n.appearance.effects, &path, bounds, &crate::GeomContext { doc, ..crate::GeomContext::of(n) });
        m.kind = NodeKind::Path { path: g, rule, live: None, clipping: false, guide: false };
        // Its gradients move with the outline, as the canvas paints them.
        if carry.moves(&m.appearance) {
            carry.apply(&mut m.appearance);
        }
        // So do the raster effects of its fills and strokes, by `n`'s Transform effects
        // ([`crate::follow_items`]) that moved the outline.
        let mut owner = std::borrow::Cow::Borrowed(n.appearance.effects.as_slice());
        if carry.skipped() {
            carry.hide_skipped(owner.to_mut());
            if m.appearance.effects == n.appearance.effects {
                let mut ap = (*m.appearance).clone();
                ap.effects = owner.to_vec();
                m.appearance = ap.into();
            }
        }
        crate::raster::follow_items_of(&mut m, &owner);
    }
    without_geometry_effects(&mut m);
    m
}

fn without_geometry_effects(n: &mut Node) {
    if n.appearance.effects.iter().any(|e| crate::is_geometry(&e.id)) {
        let mut ap = (*n.appearance).clone();
        ap.effects = crate::raster::without_geometry(&ap.effects);
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
pub fn compound_shape_area(doc: Option<&Document>, n: &Node, hook: Option<OutlineHook>) -> Option<PathData> {
    compound_shape_area_in(doc, n, hook, &SymbolNodes::default())
}

/// [`compound_shape_area`], with the symbol art given so far counted in `nodes`.
fn compound_shape_area_in(doc: Option<&Document>, n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes) -> Option<PathData> {
    if !n.is_compound_shape() {
        return None;
    }
    let mut all: Vec<Node> = vec![];
    // Each member that has a shape: its operation and how many of the leaves are its.
    let mut members: Vec<(po::BoolOp, usize)> = vec![];
    for c in n.children()? {
        let before = all.len();
        leaves(doc, c, hook, nodes, &mut all);
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
pub fn compound_shape_art(doc: Option<&Document>, n: &Node, hook: Option<OutlineHook>) -> Option<Node> {
    compound_shape_art_in(doc, n, hook, &SymbolNodes::default())
}

/// [`compound_shape_art`], with the symbol art given so far counted in `nodes`.
fn compound_shape_art_in(doc: Option<&Document>, n: &Node, hook: Option<OutlineHook>, nodes: &SymbolNodes) -> Option<Node> {
    let area = compound_shape_area_in(doc, n, hook, nodes)?;
    let mut art = Node::path(n.id, area, Appearance { items: n.appearance.items.clone(), ..Default::default() });
    (art.opacity, art.blend) = (n.opacity, n.blend);
    Some(art)
}

/// A result path styled like `src` (its appearance, opacity and blend mode).
fn styled(src: &Node, path: PathData, outline: bool) -> Node {
    let mut appearance = Appearance { items: src.appearance.items.clone(), ..Default::default() };
    if outline {
        // The edges take the paint, opacity, blend mode and overprint of the top fill that paints,
        // or of the top stroke that paints when no fill does.
        let layer = top_painting_fill(&src.appearance).or_else(|| src.appearance.items.iter().rev().find(|i| paints(i)));
        let mut stroke = StrokeLayer::new(layer.map_or(Paint::None, |i| i.paint().clone()), 1.0);
        if let Some(i) = layer {
            (stroke.opacity, stroke.blend, stroke.overprint) = (i.opacity(), i.blend(), i.overprint());
        }
        appearance = Appearance { items: vec![AppearanceItem::Stroke(stroke)], ..Default::default() };
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
/// each effect) it returns the children as they are. The swatches and patterns that live-effect
/// plug-ins painting their pieces name are looked up in `doc` ([`shape_art`]).
pub fn pathfinder_children(doc: Option<&Document>, group: &Node, hook: Option<OutlineHook>) -> Option<Vec<Arc<Node>>> {
    if !has_pathfinder(group) {
        return None;
    }
    let source: &[Arc<Node>] = group.children()?;
    // A layer's ruler guides take no part in the operation: they stay above its result.
    let art = Node::guide_run_start(source);
    if art < source.len() {
        let mut without = group.clone();
        if let Some(ch) = without.children_mut() {
            ch.truncate(art);
        }
        let mut out = pathfinder_children(doc, &without, hook)?;
        out.extend(source.iter().skip(art).cloned());
        return Some(out);
    }
    // When the group has a visible fill of its own, that fill paints its members' geometry, a
    // compound shape's area included ([`evaluate_container`]), so Remove Unpainted Artwork keeps
    // every Divide piece. Outline's open segments still go by their source's fill.
    let own_fill = has_visible_fill(group);
    // A compound shape's area has no paint of its own: the shape's own appearance paints it. The
    // shape's top painting fill is the area's fill for Remove Unpainted Artwork and for Outline's
    // strokes, while Divide pieces stay unpainted so that the fill is not drawn twice.
    let area_style = group.is_compound_shape().then(|| {
        let items = group.appearance.items.iter().filter(|i| paints(i)).cloned().collect();
        Node::path(group.id, PathData::default(), Appearance { items, ..Default::default() })
    });
    let style_of = |s: &Node| -> Option<Node> {
        let a = area_style.as_ref().filter(|_| s.id == group.id && s.appearance.items.is_empty())?;
        let mut a = a.clone();
        (a.opacity, a.blend) = (s.opacity, s.blend);
        Some(a)
    };
    let mut stack: Vec<Arc<Node>> = if group.is_compound_shape() {
        // The group's own appearance paints the area ([`evaluate_container`]), so the art has none.
        match compound_shape_area(doc, group, hook) {
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
                leaves(doc, c, hook, &nodes, &mut lv);
            }
            if nodes.spent() || !within_budget(&lv) {
                return Some(source.to_vec());
            }
            let (ids, srcs): (Vec<NodeId>, Vec<crate::trap::TrapSource>) = lv
                .iter()
                .filter_map(|l| {
                    let (path, rule) = geometry(l)?;
                    let fill = top_painting_fill(&l.appearance).map_or(Paint::None, |i| i.paint().clone());
                    Some((l.id, crate::trap::TrapSource { path, rule, fill }))
                })
                .unzip();
            let made = crate::trap::traps(&srcs, &crate::trap::TrapOptions::from_params(&e.params));
            stack.extend(made.into_iter().map(|p| Arc::new(crate::trap::trap_node(ids.get(p.from).copied().unwrap_or(NodeId(0)), p))));
            continue;
        }
        let Some(op) = op_of(&e.id) else { continue };
        let options = PathfinderEffectOptions::from_params(&e.params);
        let shape_mode =
            matches!(op, PathfinderOp::Unite | PathfinderOp::MinusFront | PathfinderOp::Intersect | PathfinderOp::Exclude | PathfinderOp::MinusBack);
        let nodes = SymbolNodes::default();
        let all: Vec<Vec<Node>> = stack
            .iter()
            .map(|c| {
                let mut lv = vec![];
                leaves(doc, c, hook, &nodes, &mut lv);
                lv
            })
            .collect();
        if nodes.spent() || !within_budget(all.iter().flatten()) {
            return Some(source.to_vec());
        }
        // (style source, shape) per stack entry; keys index `sources`.
        let mut sources: Vec<Node> = vec![];
        let mut shapes: Vec<po::Shape> = vec![];
        // Keys of the group operands whose art has no area left at the precision.
        let mut collapsed: Vec<u64> = vec![];
        for mut lv in all {
            if shape_mode && lv.len() > 1 {
                let parts: Vec<(PathData, FillRule)> = lv.iter().filter_map(geometry).collect();
                let refs: Vec<(&PathData, FillRule)> = parts.iter().map(|(p, r)| (p, *r)).collect();
                let key = sources.len() as u64;
                if let Some(top) = lv.pop() {
                    let united = po::unite_all_with(&refs, options.precision);
                    if united.is_empty() && !parts.is_empty() {
                        collapsed.push(key);
                    }
                    shapes.push(po::Shape::new(united, FillRule::NonZero, key));
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
        // An operand with no area still takes part: Intersect, and Minus Front or Minus Back when
        // it is the shape the others are cut from, then leave nothing.
        let is_collapsed = |s: &po::Shape| collapsed.contains(&s.key);
        let nothing_left = match op {
            PathfinderOp::Intersect => shapes.iter().any(is_collapsed),
            PathfinderOp::MinusFront => shapes.first().is_some_and(is_collapsed),
            PathfinderOp::MinusBack => shapes.last().is_some_and(is_collapsed),
            _ => false,
        };
        if nothing_left {
            return Some(vec![]);
        }
        shapes.retain(|s| !s.path.is_empty() && s.path.subpaths.iter().all(|sp| sp.anchors.iter().all(|a| a.p.x.is_finite() && a.p.y.is_finite())));
        if shapes.is_empty() {
            return Some(vec![]);
        }
        // Remove Redundant Points: each result loses the anchors that don't change its shape.
        let tidy = |path: PathData| if options.remove_redundant_points { po::remove_redundant_points(&path, options.precision) } else { path };
        stack = if op == PathfinderOp::SoftMix {
            let rate = mixing_rate(&e.params);
            po::regions_with(&shapes, options.precision)
                .into_iter()
                .filter(|r| !r.path.is_empty())
                .map(|r| Arc::new(soft_mix_face(tidy(r.path), &r.sources, rate, &shapes, &sources)))
                .collect()
        } else if op == PathfinderOp::HardMix {
            po::regions_with(&shapes, options.precision)
                .into_iter()
                .filter(|r| !r.path.is_empty())
                .map(|r| Arc::new(hard_mix_face(tidy(r.path), &r.sources, &shapes, &sources)))
                .collect()
        } else {
            // Divide and Outline Will Remove Unpainted Artwork: the pieces of members with no
            // visible fill go.
            let unpainted = options.remove_unpainted
                && match op {
                    PathfinderOp::Divide => !own_fill,
                    PathfinderOp::Outline => true,
                    _ => false,
                };
            let fill_sources: Vec<Option<Node>> = sources.iter().map(style_of).collect();
            let fill_source = |k: u64| fill_sources[k as usize].as_ref().unwrap_or(&sources[k as usize]);
            po::pathfinder_with(op, &shapes, options.precision)
                .into_iter()
                .filter(|r| !r.path.is_empty() && !(unpainted && !has_visible_fill(fill_source(r.key))))
                .map(|r| {
                    let outline = op == PathfinderOp::Outline;
                    let src = if outline { fill_source(r.key) } else { &sources[r.key as usize] };
                    Arc::new(styled(src, tidy(r.path), outline))
                })
                .collect()
        };
    }
    Some(stack)
}

fn mixing_rate(params: &Value) -> f64 {
    params.get("mixingRate").and_then(|v| v.as_f64()).filter(|r| r.is_finite() && *r > 0.0 && *r <= 100.0).map(|r| r / 100.0).unwrap_or(0.5)
}

fn fill_cmyk(src: &Node) -> Option<[f32; 4]> {
    // The top fill that paints: a hidden fill adds no color.
    let paint = top_painting_fill(&src.appearance)?.paint().clone();
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

/// The top fill of `a` that paints: hidden fills and fills of no paint are skipped.
fn top_painting_fill(a: &Appearance) -> Option<&AppearanceItem> {
    a.items.iter().rev().find(|i| i.is_fill() && paints(i))
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

/// Has `n` a visible fill that paints, as the Pathfinder panel's Remove Unpainted Artwork counts it?
fn has_visible_fill(n: &Node) -> bool {
    n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if f.visible && !f.paint.is_none()))
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
/// as their symbols' art from `doc`; a clip group or clipping layer (`n` itself or one among its
/// members) as the region its clipping path clips to ([`crate::clip::clip_outline`]), so members
/// it hides add nothing. Images, other symbol instances and guides add nothing. A Repeat counts the
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
                match compound_shape_area(doc, n, Some(&hook)) {
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
        // A clip group whose clipping type's strokes paint around it is outlined as that clip
        // group: the strokes outside the clip add nothing.
        if let Some(inner) = crate::reshape::painted_clip_inner(n) {
            return members(doc, inner, out);
        }
        // A clip group shows its members only inside its clipping path: its outline is that
        // region, not the clipped members' own shapes, which may reach far outside it (#7289).
        if n.clips() {
            let clip = n.children().and_then(|ch| ch.first());
            out.extend(clip.and_then(|c| crate::clip::clip_outline(c)).map(|(bp, rule)| (PathData::from_bezpath(&bp), rule)));
            return;
        }
        for c in n.children().into_iter().flatten() {
            push(doc, c, out);
        }
    }
    let mut out = vec![];
    members(doc, n, &mut out);
    out
}

/// A path node of `id` on `path` (filled under `rule`) painted by `ap`.
pub(crate) fn shape_node(id: NodeId, path: PathData, rule: FillRule, ap: Appearance) -> Node {
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

/// The members of a container that [`take_held_guides`] visits, by index, each with the same for
/// its own members. A ruler guide's entry is empty, whether its container is a layer or a group; a
/// container's is not.
#[derive(Default)]
struct HeldGuides(Vec<(usize, HeldGuides)>);

/// The members of `n` that are ruler guides (`is_guide`), and those holding one at any depth, in
/// order. Every member at every depth is looked at once: a sublayer or group with no guide of its
/// own is still looked into, and a guide directly in a group counts as one in a layer does. A
/// member holding no guide gets no entry.
fn held_guides(n: &Node, is_guide: &impl Fn(&Node) -> bool) -> HeldGuides {
    let mut held = vec![];
    for (i, c) in n.children().into_iter().flatten().enumerate() {
        if is_guide(c) {
            held.push((i, HeldGuides::default()));
            continue;
        }
        let inner = held_guides(c, is_guide);
        if !inner.0.is_empty() {
            held.push((i, inner));
        }
    }
    HeldGuides(held)
}

/// Move the ruler guides of `n` and of its members that [`held_guides`] found (`held`) to `out`:
/// those of `n` first, in order, then those of each member in turn. A guide taken from a hidden or
/// locked container, or from one inside such a container, is hidden or locked itself. Only the
/// containers in `held` are copied, and only those are visited.
fn take_held_guides(n: &mut Node, held: HeldGuides, out: &mut Vec<Arc<Node>>, hidden: bool, locked: bool) {
    let Some(ch) = n.children_mut() else { return };
    let (mut own, mut deeper) = (vec![], vec![]);
    for (i, inner) in held.0 {
        if inner.0.is_empty() {
            own.push(i);
            continue;
        }
        let Some(c) = ch.get_mut(i) else { continue };
        let c = Arc::make_mut(c);
        let (hidden, locked) = (hidden || !c.visible, locked || c.locked);
        take_held_guides(c, inner, &mut deeper, hidden, locked);
    }
    if !own.is_empty() {
        let mut own = own.into_iter().peekable();
        for (i, mut c) in std::mem::take(ch).into_iter().enumerate() {
            if own.next_if_eq(&i).is_none() {
                ch.push(c);
                continue;
            }
            if hidden || locked {
                let g = Arc::make_mut(&mut c);
                g.visible &= !hidden;
                g.locked |= locked;
            }
            out.push(c);
        }
    }
    out.append(&mut deeper);
}

/// Move the ruler guides of `n` and those held at any depth by the sublayers and groups among its
/// members to `out`, in order: a container's own guides, then those of its members. A sublayer or
/// group with no guide of its own still gives up those of its members. A guide taken from a hidden
/// or locked sublayer or group, or from one inside such a container, is hidden or locked itself.
/// Each member at every depth is looked at once (`is_guide`), and only containers holding a guide
/// are copied.
fn take_sublayer_guides(n: &mut Node, out: &mut Vec<Arc<Node>>, is_guide: &impl Fn(&Node) -> bool) {
    let held = held_guides(n, is_guide);
    take_held_guides(n, held, out, false, false);
}

struct PreparedContainer {
    m: Node,
    guides: Vec<Arc<Node>>,
    paint: bool,
    carry: crate::Carry,
}

/// The existing container pass up to its own rows becoming art. Keep its carried appearance for
/// rendering, and retain the same Carry for freezing only missing stored placements separately.
fn prepare_container(doc: Option<&Document>, n: &Node) -> Option<PreparedContainer> {
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
    let mut carry = crate::Carry::default();
    // Its ruler guides take no part: they go back above the evaluated art at the end. With effects
    // that replace or copy the art, so do its guides among the art and those in its sublayers and
    // groups, which would otherwise go with that art: they stay on `n`, in order, hidden and
    // locked as their containers were.
    let mut guides = vec![];
    if pathfinder || !fx.is_empty() {
        take_sublayer_guides(&mut m, &mut guides, &Node::is_ruler_guide);
    } else if let Some(ch) = m.children_mut() {
        guides = ch.split_off(Node::guide_run_start(ch));
    }
    if pathfinder {
        let hook = |c: &Node| crate::outline_object(doc, c);
        let result = pathfinder_children(doc, &m, Some(&hook)).unwrap_or_default();
        if let Some(ch) = m.children_mut() {
            *ch = result;
        }
        // The area is art now: drawn again, it is no compound shape of its own.
        m.extras_mut().compound_shape = false;
    }
    if !fx.is_empty() {
        m = crate::reshape::outlined_members(doc, &m);
        {
            // The members' raster effects then follow the group's own Transform effects as well.
            let _later = crate::raster::LaterGroups::enter(&n.appearance.effects);
            members_as_drawn(doc, &mut m);
        }
        carry = crate::reshape::reshape_leaves_carrying(&mut m, &fx, &n.appearance.effects, doc);
        // The group's own fills and strokes paint the members as a whole: their gradients follow
        // the effects as the members' do.
        if paint && carry.moves(&m.appearance) {
            carry.apply(&mut m.appearance);
        }
    }
    m.appearance.effects = crate::raster::without_geometry(&m.appearance.effects);
    m.appearance.effects.retain(|e| !is_pathfinder(&e.id));
    Some(PreparedContainer { m, guides, paint, carry })
}

/// `n`, a group or layer, as plain art: its Pathfinder effects evaluated, its geometry effects
/// reshaping all its members as one piece (the whole art's bounds as their reference box) after
/// the members' own geometry and Pathfinder effects ([`members_as_drawn`]), and its
/// own fills and strokes turned into art painting the members' geometry ([`member_shapes`]) below
/// the members (after a clipping path) or above them, as its contents slot says. Its
/// transparency, opacity mask and raster effects stay. `None` when it has none of these. Live
/// objects among the members are evaluated with the symbols of `doc` (see [`member_shapes`]), and
/// a Pathfinder operation or compound shape takes the symbol instances among its members as their
/// symbols' art from `doc`. A Transform effect that makes copies with raster effects listed before
/// it gives each copy those raster effects ([`copied_container`]).
pub fn evaluate_container(doc: Option<&Document>, n: &Node) -> Option<Node> {
    if !is_container(n) {
        return None;
    }
    if let Some(t) = crate::reshape::copied_container_raster_at(n) {
        return Some(copied_container(doc, n, t));
    }
    let PreparedContainer { mut m, guides, paint, .. } = prepare_container(doc, n)?;
    if paint {
        // A clip group whose clipping type paints strokes is now a group of a clip group and those
        // strokes ([`crate::reshape::paint_type_clip`]): its own fills and strokes paint inside
        // that clip group. Such groups among the members are measured as their clip groups.
        let wrapped = n.clips() && crate::reshape::painted_clip_inner(&m).is_some();
        let (shapes, bounds) = (member_shapes(doc, &m), crate::reshape::clip_bounds(&m));
        let art =
            |items: &[AppearanceItem]| -> Vec<Arc<Node>> { items.iter().filter_map(|i| item_art(n.id, i, &shapes, bounds)).map(Arc::new).collect() };
        let (below, above) = m.appearance.split_contents();
        let (below, above) = (art(below), art(above));
        let at = usize::from(wrapped || m.clips());
        let target = if wrapped { m.children_mut().and_then(|ch| ch.first_mut()).map(Arc::make_mut) } else { Some(&mut m) };
        if let Some(ch) = target.and_then(Node::children_mut) {
            let at = at.min(ch.len());
            ch.splice(at..at, below);
            ch.extend(above);
        }
    }
    m.appearance.items.clear();
    m.appearance.contents_index = None;
    if let Some(ch) = m.children_mut() {
        ch.extend(guides);
    }
    Some(m)
}

/// The exact pre-copy source shared by rendering and stored gradient placement.
fn copied_container_source(doc: Option<&Document>, n: &Node, t: usize) -> (Node, Vec<Arc<Node>>, Node) {
    let mut m = n.clone();
    let mut guides = vec![];
    take_sublayer_guides(&mut m, &mut guides, &Node::is_ruler_guide);
    let mut source = m.clone();
    source.appearance.effects = n.appearance.effects.get(..t).unwrap_or_default().to_vec();
    // With no geometry effect before the Transform effect, `evaluate_container` leaves the members'
    // own effects live (it only paints the group's own fills and strokes): the members are drawn
    // first, so their effects apply before the copies are made.
    if crate::reshape::geometry_effects(&source).is_empty() {
        source = crate::reshape::outlined_members(doc, &source);
        // The members' raster effects follow the group's own Transform effects as well.
        let _later = crate::raster::LaterGroups::enter(&n.appearance.effects);
        members_as_drawn(doc, &mut source);
    }
    (m, guides, source)
}

/// `n`, a group or layer whose own effects split at `t` ([`crate::reshape::copied_container_raster_at`]),
/// as plain art: the effects before the Transform effect at `t` evaluated on its members as one
/// piece ([`evaluate_container`]), with the raster effects among them, and that piece copied by the
/// Transform effect, each copy a group of its own carrying those raster effects moved, turned,
/// reflected and scaled with it ([`crate::reshape::copied_container_art`]). The art keeps the kind,
/// transparency and opacity mask of `n` and the raster effects from `t` on, which paint once around
/// all the copies; a clip group's or clipping layer's copies each clip, and the art itself does not.
/// Its ruler guides, those in its sublayers and groups included, go once above the copies, hidden
/// and locked as their containers were.
fn copied_container(doc: Option<&Document>, n: &Node, t: usize) -> Node {
    let (mut m, guides, source) = copied_container_source(doc, n, t);
    let mut piece = evaluate_container(doc, &source).unwrap_or_else(|| {
        let mut p = source.clone();
        p.appearance.effects = crate::raster::without_geometry(&p.appearance.effects);
        p
    });
    // The piece is one copy: the transparency, opacity mask and guides stay on the art.
    piece.name = None;
    piece.visible = true;
    piece.locked = false;
    (piece.opacity, piece.blend, piece.isolate) = (1.0, Default::default(), false);
    piece.extras_mut().mask = None;
    // The copies are made about the piece's bounds, measured first: a layer leaves its hidden
    // members out of them, and a group does not.
    let bounds = piece.geometric_bounds();
    // A layer's copies are groups in it.
    if let NodeKind::Layer { children, clip, .. } = &mut piece.kind {
        piece.kind = NodeKind::Group { children: std::mem::take(children), clip: *clip };
    }
    let (art, effects) = crate::reshape::copied_container_art(doc, n, t, piece, bounds);
    let copies = art.children().cloned().unwrap_or_default();
    if let NodeKind::Group { children, clip } | NodeKind::Layer { children, clip, .. } = &mut m.kind {
        *children = copies;
        *clip = false;
    }
    m.appearance.items.clear();
    m.appearance.contents_index = None;
    m.appearance.effects = effects;
    if let Some(ch) = m.children_mut() {
        ch.extend(guides);
    }
    m
}

/// Freeze representable fitted own paints to the box the container pass gives them, retaining
/// every live effect and already explicit placement. Hidden-only containers the pass does not
/// evaluate stay unchanged; this does not add a different fallback to `evaluate_container`.
pub fn pinned_container_appearance(doc: &Document, n: &Node) -> Option<Appearance> {
    if !n.appearance.has_unplaced_gradient() {
        return None;
    }
    if let Some(t) = crate::reshape::copied_container_raster_at(n) {
        let (_, _, source) = copied_container_source(Some(doc), n, t);
        let pinned = pinned_container_appearance(doc, &source)?;
        let mut out = (*n.appearance).clone();
        out.items = pinned.items;
        return Some(out);
    }
    let PreparedContainer { m, carry, .. } = prepare_container(Some(doc), n)?;
    let bounds = m.geometric_bounds()?;
    let mut out = (*n.appearance).clone();
    let mut changed = false;
    for (item, original) in out.items.iter_mut().zip(&n.appearance.items) {
        if paints(original)
            && matches!(original.paint(), Paint::Gradient(g) if !g.is_placed())
            && let Some(pinned) = crate::pinned_gradient_item(original, bounds, &[&carry])
        {
            *item = pinned;
            changed = true;
        }
    }
    changed.then_some(out)
}

/// Replace each visible member of `n` with geometry or Pathfinder effects of its own, at every
/// depth of its groups and layers, by the art it draws ([`evaluate_container`],
/// [`crate::reshape::own_effects_art`]), so the geometry effects of `n` reshape what its members
/// draw. A clip group's clipping path only takes the geometry effects in it into its geometry
/// ([`crate::clip::with_clip_effects_applied_while`]), and the members of a clipping group or
/// compound path count toward the limit below. A group or layer in a clipping path that has
/// geometry effects of its own is walked twice, once for its members' effects and once for its
/// own, and its members count each time. Images, symbol instances and the other objects
/// that `own_effects_art` leaves alone stay as they are, and so does a group or layer with a 3D
/// effect with everything in it. Calls nested in one another, through the
/// groups and layers it walks and the art it evaluates (symbol art and Pathfinder members
/// included), stop at [`vectorcraft_doc::depth::MAX_DEPTH`] deep, and together they replace at
/// most [`crate::SYMBOL_ART_MAX_NODES`] members: past either limit, members stay as they are.
pub(crate) fn members_as_drawn(doc: Option<&Document>, n: &mut Node) {
    let Some(_frame) = DrawnFrame::enter() else { return };
    let clip = usize::from(n.clips());
    let Some(ch) = n.children_mut() else { return };
    // The clipping path's own geometry effects shape it before the effects around it do.
    if clip == 1
        && let Some(first) = ch.first_mut()
        && let Some(applied) = crate::clip::with_clip_effects_applied_while(first, DrawnFrame::take_member)
    {
        *first = Arc::new(applied);
    }
    for c in ch.iter_mut().skip(clip).filter(|c| c.visible) {
        if !DrawnFrame::take_member() {
            return;
        }
        // A group or layer with a 3D effect stays whole: the geometry effects listed after its 3D
        // effect reshape its 3D art.
        if is_container(c) && (crate::has_solid(c) || crate::has_revolve(c)) {
            continue;
        }
        let art = if !is_container(c) {
            crate::reshape::own_effects_art(doc, c)
        } else if has_pathfinder(c) || !crate::reshape::geometry_effects(c).is_empty() {
            evaluate_container(doc, c)
        } else {
            None
        };
        match art {
            Some(a) => *c = Arc::new(a),
            None if is_container(c) => members_as_drawn(doc, Arc::make_mut(c)),
            None => {}
        }
    }
}

thread_local! {
    /// How many [`members_as_drawn`] calls are running, one inside another, and how many members
    /// they have visited since the outermost one started.
    static DRAWN: std::cell::Cell<(usize, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// One running [`members_as_drawn`] call, counted in [`DRAWN`] until it is dropped.
struct DrawnFrame;

impl DrawnFrame {
    /// Count one more call; `None` when [`vectorcraft_doc::depth::MAX_DEPTH`] are running. The
    /// outermost call starts the count of members again.
    fn enter() -> Option<Self> {
        let (frames, members) = DRAWN.with(|d| d.get());
        if frames >= vectorcraft_doc::depth::MAX_DEPTH {
            return None;
        }
        DRAWN.with(|d| d.set((frames + 1, if frames == 0 { 0 } else { members })));
        Some(DrawnFrame)
    }

    /// Count one more member visited; `false` once [`crate::SYMBOL_ART_MAX_NODES`] have been.
    fn take_member() -> bool {
        let (frames, members) = DRAWN.with(|d| d.get());
        if members >= crate::SYMBOL_ART_MAX_NODES {
            return false;
        }
        DRAWN.with(|d| d.set((frames, members + 1)));
        true
    }
}

impl Drop for DrawnFrame {
    fn drop(&mut self) {
        DRAWN.with(|d| {
            let (frames, members) = d.get();
            d.set((frames.saturating_sub(1), members));
        });
    }
}

#[cfg(test)]
mod tests;
