use super::params::{is_geometry, merged_params};
use super::{
    Appearance, AppearanceItem, BezPath, Effect, FillRule, Node, NodeKind, PathData, Rect, StrokeLayer, Value, WarpStyle, distort, stroke, stylize,
    util, warp,
};
use vectorcraft_doc::Document;

/// Context the geometry effects may need from the object's appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeomContext<'a> {
    /// The stroke `path.outlineStroke` outlines (its weight, caps, joins, dashes, alignment,
    /// profile and arrowheads): the stroke the effect sits on, else the object's top painted
    /// stroke. `None` outlines a plain 1 pt stroke.
    pub stroke: Option<&'a StrokeLayer>,
    /// The object's fill rule (inside and outside alignment).
    pub rule: FillRule,
    /// The seed of the Transform effect's Random: the object's id, so each object varies its own
    /// way and keeps its result on every redraw.
    pub seed: u64,
    /// The object's appearance, which live-effect plug-ins are given read only (their fills and
    /// strokes).
    pub appearance: Option<&'a Appearance>,
    /// The index in `appearance.items` of the fill or stroke whose own effects these are; `None`:
    /// the object's effects.
    pub item: Option<usize>,
    /// The document that the swatches and patterns named by a live-effect plug-in that paints its
    /// pieces are looked up in. Without one, a result that names any is refused.
    pub doc: Option<&'a Document>,
}

impl<'a> GeomContext<'a> {
    /// The context of `n`'s object-level effects.
    pub fn of(n: &'a Node) -> Self {
        let rule = match &n.kind {
            NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
            _ => FillRule::NonZero,
        };
        let stroke = n.appearance.stroke().filter(|s| !s.paint.is_none() && s.width > 0.0);
        Self { stroke, rule, seed: n.id.0, appearance: Some(&n.appearance), item: None, doc: None }
    }

    /// The context of the effects on `item`, one of the same object's appearance items: a
    /// stroke's own effects outline that stroke, and a plug-in is told which item it is on.
    pub fn item(self, item: &'a AppearanceItem) -> Self {
        let at = self.appearance.and_then(|a| a.items.iter().position(|i| std::ptr::eq(i, item)));
        match item {
            AppearanceItem::Stroke(s) => Self { stroke: Some(s), item: at, ..self },
            AppearanceItem::Fill(_) => Self { item: at, ..self },
        }
    }
}

/// Most anchors a chain of geometry effects makes of one path, or of all the paths of a type,
/// symbol or image art together; a path that already has more keeps its own count as the limit.
/// Roughen resamples each segment into up to 2000 points, Zig Zag into up to 101 and the warps
/// into up to 64, so stacked effects multiply: three Roughens of a rectangle needed tens of
/// gigabytes (#3578). Each effect builds within it, and one whose result would still pass it is
/// left out.
pub(crate) const MAX_EFFECT_ANCHORS: usize = 2_000_000;

/// Most lines the renderer is estimated to draw a chain of geometry effects' result with at
/// 1 px/pt ([`util::line_estimate`]), or the path's own estimate when that is more. A large
/// Roughen or Zig Zag makes segments thousands of points long, each drawn as up to hundreds of
/// lines: two million anchors of them came to 141 million lines, 7 GB in the renderer and past
/// the 2²⁷ lines vello takes of one path (#3578). Art of small curves draws its anchors as a few
/// lines each, so this rarely binds before [`MAX_EFFECT_ANCHORS`] does.
pub(crate) const MAX_EFFECT_LINES: usize = 8_000_000;

/// What geometry effects may make of a path, or what a path costs: its anchors and the lines the
/// renderer is estimated to draw it with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Budget {
    pub anchors: usize,
    pub lines: usize,
}

impl Budget {
    /// The budget of a chain of geometry effects.
    pub(crate) const EFFECTS: Self = Self { anchors: MAX_EFFECT_ANCHORS, lines: MAX_EFFECT_LINES };

    /// What `path` costs.
    pub(crate) fn of(path: &PathData) -> Self {
        Self { anchors: path.anchor_count(), lines: util::line_estimate(path) }
    }

    /// This budget, raised to `cost` where that is more.
    pub(crate) fn at_least(self, cost: Self) -> Self {
        Self { anchors: self.anchors.max(cost.anchors), lines: self.lines.max(cost.lines) }
    }

    /// Does `cost` fit this budget?
    pub(crate) fn fits(self, cost: Self) -> bool {
        cost.anchors <= self.anchors && cost.lines <= self.lines
    }

    /// What is left of this budget once `cost` is spent.
    pub(crate) fn minus(self, cost: Self) -> Self {
        Self { anchors: self.anchors.saturating_sub(cost.anchors), lines: self.lines.saturating_sub(cost.lines) }
    }

    /// This cost and `cost` together.
    pub(crate) fn plus(self, cost: Self) -> Self {
        Self { anchors: self.anchors.saturating_add(cost.anchors), lines: self.lines.saturating_add(cost.lines) }
    }
}

/// Evaluate the visible geometry effects of `effects` on `path`, in order. `bounds` is the
/// reference box of the first effect (usually the object's geometric bounds); each later effect
/// uses the bounds of its input. Raster and unknown effects are skipped, and so is an effect
/// whose result would pass 2 million anchors or 8 million lines drawn (or the path's own counts,
/// when they are more).
pub fn apply_geometry(effects: &[Effect], path: &PathData, bounds: Rect) -> PathData {
    apply_geometry_with(effects, path, bounds, &GeomContext::default())
}

/// [`apply_geometry`] with an explicit [`GeomContext`].
pub fn apply_geometry_with(effects: &[Effect], path: &PathData, bounds: Rect, ctx: &GeomContext) -> PathData {
    apply_geometry_within(effects, path, bounds, ctx, Budget::EFFECTS)
}

/// Does effect `e` safely commute with pure translation when it is terminal?
/// Only positive offsets with round joins commute exactly with pure translations for non-zero fills (#4145).
fn is_terminal_offset_eligible(e: &Effect) -> bool {
    if e.id != "path.offsetPath" {
        return false;
    }
    let p = merged_params(&e.id, &e.params);
    util::join(&p, "joins") == vectorcraft_pathops::Join::Round && util::num(&p, "offset", 10.0) >= 0.0
}

/// Optimize a list of visible geometry effects: move pure-translation Transform effects with
/// copies after a terminal Offset Path effect when the effective style is safe (NonZero fill and
/// no stroke), so that the offset path is evaluated once on the base path rather than on dozens
/// of overlapping copies, avoiding combinatorial explosions in the planar sweep (#4145).
pub(crate) fn optimize_geometry_effects<'a>(effects: &[&'a Effect], style_safe: bool) -> Vec<&'a Effect> {
    let mut out: Vec<&'a Effect> = effects.to_vec();
    if !style_safe || out.len() < 2 {
        return out;
    }
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..out.len().saturating_sub(1) {
            let e1 = out[i];
            if e1.id == "distort.transform" {
                let p1 = merged_params(&e1.id, &e1.params);
                if let Some((_, copies)) = crate::distort::transform_move(&p1)
                    && copies > 0
                    && is_terminal_offset_eligible(out[i + 1])
                    && out.get(i + 2..).is_some_and(|tail| tail.iter().all(|e| e.id == "distort.transform"))
                {
                    out.swap(i, i + 1);
                    changed = true;
                }
            }
        }
    }
    out
}

/// [`apply_geometry_with`] whose effects keep to `budget`, raised to what `path` costs where that
/// is more.
pub(crate) fn apply_geometry_within(effects: &[Effect], path: &PathData, bounds: Rect, ctx: &GeomContext, budget: Budget) -> PathData {
    apply_geometry_carrying_within(effects, path, bounds, ctx, budget, None)
}

/// [`apply_geometry_with`], and the [`Carry`] that moves the object's gradients along with the
/// effects that move the object as a whole.
pub fn apply_geometry_carrying(effects: &[Effect], path: &PathData, bounds: Rect, ctx: &GeomContext) -> (PathData, Carry) {
    let mut carry = Carry::default();
    let out = apply_geometry_carrying_within(effects, path, bounds, ctx, Budget::EFFECTS, Some(&mut carry));
    (out, carry)
}

/// [`apply_geometry_carrying`] on kurbo paths.
pub fn apply_geometry_bez_carrying(effects: &[Effect], path: &BezPath, bounds: Rect, ctx: &GeomContext) -> (BezPath, Carry) {
    let (out, carry) = apply_geometry_carrying(effects, &PathData::from_bezpath(path), bounds, ctx);
    (out.to_bezpath(), carry)
}

/// How placed gradients follow geometry effects. The Transform effect without copies maps them as
/// it maps the object (with copies, `crate::reshape` gives each copy its own). Free Distort moves
/// and scales them from the object's bounds before the effect to its bounds after it. For the
/// other geometry effects it is empty. It does not change gradients fitted to the object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Carry(Vec<CarryStep>);

#[derive(Clone, Copy, Debug, PartialEq)]
enum CarryStep {
    Affine(vectorcraft_geom::Affine),
    /// From box `from` to box `to` (strokes: their stroke-inflated boxes).
    Boxes {
        from: Rect,
        to: Rect,
    },
}

impl Carry {
    /// Does effect `id` with merged params `p` on geometry with reference box `b` (`seed` for
    /// Random) move placed gradients? Free Distort does unless it leaves every corner in place;
    /// the Transform effect does without copies (with copies the original stays where it is, and
    /// `crate::reshape` gives each copy its own gradients).
    pub(crate) fn follows(id: &str, p: &Value, b: Rect, seed: u64) -> bool {
        match id {
            "distort.transform" => {
                let (m, copies) = distort::transform_map(b, p, seed);
                copies == 0 && m != vectorcraft_geom::Affine::IDENTITY && m.is_finite()
            }
            "distort.freeDistort" => {
                let corners = distort::free_distort_corners(b, p);
                let tol = 1e-9 * b.width().hypot(b.height()).max(1.0);
                let still =
                    [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)].iter().zip(&corners).all(|(a, c)| c.distance((*a).into()) <= tol);
                !still && corners.iter().all(|c| c.x.is_finite() && c.y.is_finite())
            }
            _ => false,
        }
    }

    /// How placed gradients follow effect `id` (as [`Self::follows`]) on an object whose geometric
    /// bounds are `from` before the effect and `to` after it; empty when they stay in place.
    pub(crate) fn of(id: &str, p: &Value, b: Rect, from: Option<Rect>, to: Option<Rect>, seed: u64) -> Carry {
        if !Self::follows(id, p, b, seed) {
            return Carry::default();
        }
        let step = match id {
            "distort.transform" => Some(CarryStep::Affine(distort::transform_map(b, p, seed).0)),
            _ => match (from, to) {
                (Some(from), Some(to)) if from != to && [to.x0, to.y0, to.x1, to.y1].iter().all(|v| v.is_finite()) => {
                    Some(CarryStep::Boxes { from, to })
                }
                _ => None,
            },
        };
        Carry(step.into_iter().collect())
    }

    /// Does it leave gradients where they are?
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Does it move any of the gradients of `ap` (those drawn at a place in document space, a
    /// freeform gradient's own points included)? Callers check first, so an appearance shared with
    /// other objects is not copied for nothing.
    pub fn moves(&self, ap: &Appearance) -> bool {
        !self.0.is_empty() && ap.has_positioned_gradient()
    }

    /// Move the placed gradients of `ap` along, through each step in turn.
    pub fn apply(&self, ap: &mut Appearance) {
        if !self.moves(ap) {
            return;
        }
        for s in &self.0 {
            match *s {
                CarryStep::Affine(m) => ap.transform_gradients(m),
                CarryStep::Boxes { from, to } => crate::reshape::follow_boxes(ap, from, to),
            }
        }
    }

    pub(crate) fn push(&mut self, more: Carry) {
        self.0.extend(more.0);
    }
}

/// [`apply_geometry_within`], adding to `carry` how gradients follow each effect.
pub(crate) fn apply_geometry_carrying_within(
    effects: &[Effect],
    path: &PathData,
    bounds: Rect,
    ctx: &GeomContext,
    budget: Budget,
    mut carry: Option<&mut Carry>,
) -> PathData {
    // `budget` raised to what `path` costs, measured once, and only for a path that has geometry
    // effects.
    let has_stroke = match (ctx.item, ctx.appearance) {
        (Some(idx), Some(app)) => match app.items.get(idx) {
            Some(AppearanceItem::Stroke(s)) => !s.paint.is_none() && s.width > 0.0,
            Some(AppearanceItem::Fill(_)) => false,
            None => ctx.stroke.is_some_and(|s| !s.paint.is_none() && s.width > 0.0),
        },
        _ => ctx.stroke.is_some_and(|s| !s.paint.is_none() && s.width > 0.0),
    };
    let style_safe = ctx.rule == FillRule::NonZero && !has_stroke;
    let visible_fx: Vec<&Effect> = effects.iter().filter(|e| e.visible && is_geometry(&e.id)).collect();
    let fx = optimize_geometry_effects(&visible_fx, style_safe);
    let mut limit = None;
    let mut cur = path.clone();
    let mut b = bounds;
    let mut first = true;
    for e in fx {
        if !first {
            b = cur.bounds().unwrap_or(b);
        }
        first = false;
        if cur.is_empty() {
            break;
        }
        let p = merged_params(&e.id, &e.params);
        let limit = *limit.get_or_insert_with(|| budget.at_least(Budget::of(path)));
        if let Some(next) = apply_one(&e.id, &p, &cur, b, ctx, limit) {
            if let Some(c) = carry.as_deref_mut() {
                c.push(Carry::of(&e.id, &p, b, cur.bounds(), next.bounds(), ctx.seed));
            }
            cur = next;
        }
    }
    cur
}

/// Convenience wrapper on kurbo paths.
pub fn apply_geometry_bez(effects: &[Effect], path: &BezPath, bounds: Rect, ctx: &GeomContext) -> BezPath {
    apply_geometry_with(effects, &PathData::from_bezpath(path), bounds, ctx).to_bezpath()
}

/// Geometry effect `id` with params `p` on `path`, whose reference box is `b`; `None` when its
/// result would not fit `budget` (the effects that multiply anchors build within its anchors).
pub(crate) fn apply_one(id: &str, p: &Value, path: &PathData, b: Rect, ctx: &GeomContext, budget: Budget) -> Option<PathData> {
    let out = apply_unchecked(id, p, path, b, ctx, budget.anchors);
    let cost = Budget::of(&out);
    if budget.fits(cost) {
        return Some(out);
    }
    if cost.lines <= budget.lines || !builds_within(id) {
        return None;
    }
    // Anchors drawn as many lines each (a large Roughen or Zig Zag): once more with as many fewer
    // anchors as there were lines too many, and a tenth to spare, as segments vary in their lines.
    let fewer = (cost.anchors as f64 * 0.9 * (budget.lines as f64 / cost.lines as f64)) as usize;
    let out = apply_unchecked(id, p, path, b, ctx, fewer.min(budget.anchors));
    budget.fits(Budget::of(&out)).then_some(out)
}

/// Does geometry effect `id` build within the anchors it is given?
fn builds_within(id: &str) -> bool {
    matches!(id, "distort.freeDistort" | "distort.roughen" | "distort.transform" | "distort.twist" | "distort.zigZag") || id.starts_with("warp.")
}

/// [`apply_one`] whose result may pass `max` when the effect doesn't build within it.
fn apply_unchecked(id: &str, p: &Value, path: &PathData, b: Rect, ctx: &GeomContext, max: usize) -> PathData {
    use util::{join, num};
    if let Some(plugin) = vectorcraft_plugins::effect::plugin_id(id) {
        let paints = vectorcraft_plugins::effect::Paints { appearance: ctx.appearance, item: ctx.item };
        return vectorcraft_plugins::effect::apply(plugin, p, path, b, ctx.rule, paints, ctx.doc).unwrap_or_else(|| path.clone());
    }
    match id {
        "distort.freeDistort" => distort::free_distort(path, b, p, max),
        "distort.puckerBloat" => distort::pucker_bloat(path, b, num(p, "amount", 0.0)),
        "distort.roughen" => distort::roughen(path, b, p, max),
        "distort.transform" => distort::transform_within(path, b, p, ctx.seed, max),
        "distort.tweak" => distort::tweak(path, b, p),
        "distort.twist" => distort::twist(path, b, num(p, "angle", 10.0), max),
        "distort.zigZag" => distort::zig_zag(path, b, p, max),
        "path.offsetPath" => {
            let off = num(p, "offset", 10.0).clamp(-1e5, 1e5);
            if off.abs() < 1e-9 {
                return path.clone();
            }
            vectorcraft_pathops::offset_path(path, off, join(p, "joins"), num(p, "miterLimit", 4.0).clamp(1.0, 500.0))
        }
        // Outline Object returns the geometry unchanged. Type reaches every geometry effect as its
        // glyph outlines (`reshape::reshape`), so type that carries it is drawn as those outlines.
        "path.outlineObject" => path.clone(),
        "path.outlineStroke" => {
            let fallback;
            let base = match ctx.stroke {
                Some(s) => s,
                None => {
                    fallback = StrokeLayer::new(vectorcraft_doc::color::Paint::None, 1.0);
                    &fallback
                }
            };
            let st = match p.get("width").and_then(Value::as_f64) {
                Some(w) => std::borrow::Cow::Owned(StrokeLayer { width: w, ..base.clone() }),
                None => std::borrow::Cow::Borrowed(base),
            };
            if st.width.is_nan() || st.width <= 0.0 {
                return path.clone();
            }
            stroke::outline_region(path, ctx.rule, &st)
        }
        "convertToShape.rectangle" | "convertToShape.roundedRectangle" | "convertToShape.ellipse" => stylize::convert_to_shape(id, b, p),
        "stylize.roundCorners" => stylize::round_corners(path, num(p, "radius", 10.0)),
        "stylize.scribble" => stylize::scribble(path, b, p),
        _ if id.starts_with(crate::addons::path::PREFIX) => crate::addons::path::apply(id, p, path, max),
        _ => match id.strip_prefix("warp.").and_then(WarpStyle::from_id) {
            Some(style) => warp::warp(path, b, style, p, max),
            None => path.clone(),
        },
    }
}
