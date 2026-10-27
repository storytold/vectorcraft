use super::params::{is_geometry, merged_params};
use super::*;

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
}

impl<'a> GeomContext<'a> {
    /// The context of `n`'s object-level effects.
    pub fn of(n: &'a Node) -> Self {
        let rule = match &n.kind {
            NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
            _ => FillRule::NonZero,
        };
        let stroke = n.appearance.stroke().filter(|s| !s.paint.is_none() && s.width > 0.0);
        Self { stroke, rule, seed: n.id.0, appearance: Some(&n.appearance), item: None }
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

/// [`apply_geometry_with`] whose effects keep to `budget`, raised to what `path` costs where that
/// is more.
pub(crate) fn apply_geometry_within(effects: &[Effect], path: &PathData, bounds: Rect, ctx: &GeomContext, budget: Budget) -> PathData {
    // `budget` raised to what `path` costs, measured once, and only for a path that has geometry
    // effects.
    let mut limit = None;
    let mut cur = path.clone();
    let mut b = bounds;
    let mut first = true;
    for e in effects.iter().filter(|e| e.visible && is_geometry(&e.id)) {
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
    use util::*;
    if let Some(plugin) = vectorcraft_plugins::effect::plugin_id(id) {
        let paints = vectorcraft_plugins::effect::Paints { appearance: ctx.appearance, item: ctx.item };
        return vectorcraft_plugins::effect::apply(plugin, p, path, b, ctx.rule, paints).unwrap_or_else(|| path.clone());
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
        _ => match id.strip_prefix("warp.").and_then(WarpStyle::from_id) {
            Some(style) => warp::warp(path, b, style, p, max),
            None => path.clone(),
        },
    }
}
