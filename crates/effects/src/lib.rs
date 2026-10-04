//! VectorCraft live effects (Illustrator's Effect menu).
//!
//! Effects live in appearance stacks as [`Effect`] `{id, params, visible}` records. This crate
//! interprets them:
//!
//! - **Geometry effects** (Distort & Transform, Path, Convert to Shape, Round Corners, Scribble,
//!   Warp) rewrite a path: [`apply_geometry`] evaluates them in stack order.
//! - **Raster effects** (Drop Shadow, Inner/Outer Glow, Feather, Gaussian Blur) are described by
//!   [`raster_effects`] and painted by the renderer; [`outset`] says how far they reach beyond
//!   the geometry.
//! - **Stroke geometry** ([`stroke`]): arrowheads, dash patterns and width profiles, shared by the
//!   renderer, the exporters and Outline Stroke.
//! - [`effect_catalog`] lists every effect with its menu path, parameter documentation and the
//!   defaults of Illustrator's dialogs. Missing parameters always fall back to those defaults.
//! - [`reshape`] applies geometry effects to type, images, symbol instances and live objects
//!   through their outlines.
//! - [`clip_outline`] is the region a clip group clips to, shared by the renderer and the SVG and
//!   PDF writers.
//!
//! Everything is deterministic: "random" effects (Roughen, Tweak, Scribble) use a seeded hash
//! noise (`seed` parameter, default 0).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bake;
mod clip;
mod distort;
mod group;
mod raster;
mod reshape;
pub mod stroke;
mod stylize;
mod util;
mod warp;

#[cfg(test)]
mod tests;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{AppearanceItem, Effect, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{BezPath, FillRule, PathData, Rect};

pub use bake::{bake_document, needs_bake};
pub use clip::clip_outline;
pub use group::{OutlineHook, PATHFINDER_EFFECTS, has_pathfinder, is_pathfinder, pathfinder_children};
pub use raster::{RasterFx, outset, raster_effects};
pub use reshape::{needs_outline, outline_art, outline_text, reshape};
pub use warp::{WarpStyle, warp_point};

/// Catalogue entry for one effect.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectInfo {
    /// Stable id, e.g. `distort.roughen`.
    pub id: &'static str,
    /// Menu label (with the ellipsis when the effect has a dialog).
    pub label: &'static str,
    /// Effect-menu path, e.g. `["Effect", "Distort & Transform"]`.
    pub menu: &'static [&'static str],
    /// Human/agent-readable parameter documentation.
    pub params: &'static str,
    /// Dialog defaults.
    pub defaults: Value,
    /// Raster (painted) rather than geometry effect.
    pub raster: bool,
}

const WARP_DOC: &str =
    "{bend: % (-100..100, 50), horizontal: % distortion (0), vertical: % distortion (0), orientation: \"horizontal\"|\"vertical\"}";
const DT: &[&str] = &["Effect", "Distort & Transform"];
const PATH: &[&str] = &["Effect", "Path"];
const SHAPE: &[&str] = &["Effect", "Convert to Shape"];
const STYLIZE: &[&str] = &["Effect", "Stylize"];
const WARP: &[&str] = &["Effect", "Warp"];
const BLUR: &[&str] = &["Effect", "Blur"];
const PATHFINDER: &[&str] = &["Effect", "Pathfinder"];

/// The warp styles in Illustrator's Style menu order: (id suffix, label).
pub const WARP_STYLES: [(&str, &str); 15] = [
    ("arc", "Arc…"),
    ("arcLower", "Arc Lower…"),
    ("arcUpper", "Arc Upper…"),
    ("arch", "Arch…"),
    ("bulge", "Bulge…"),
    ("shellLower", "Shell Lower…"),
    ("shellUpper", "Shell Upper…"),
    ("flag", "Flag…"),
    ("wave", "Wave…"),
    ("fish", "Fish…"),
    ("rise", "Rise…"),
    ("fisheye", "Fisheye…"),
    ("inflate", "Inflate…"),
    ("squeeze", "Squeeze…"),
    ("twist", "Twist…"),
];

/// Every supported effect with its dialog defaults.
pub fn effect_catalog() -> Vec<EffectInfo> {
    let g = |id, label, menu, params, defaults| EffectInfo { id, label, menu, params, defaults, raster: false };
    let r = |id, label, menu, params, defaults| EffectInfo { id, label, menu, params, defaults, raster: true };
    let mut v = vec![
        g(
            "convertToShape.rectangle",
            "Rectangle…",
            SHAPE,
            "{relative: bool (true), extraW: pt (18), extraH: pt (18), width: pt (absolute, 100), height: pt (absolute, 100)}",
            json!({"relative": true, "extraW": 18.0, "extraH": 18.0, "width": 100.0, "height": 100.0}),
        ),
        g(
            "convertToShape.roundedRectangle",
            "Rounded Rectangle…",
            SHAPE,
            "{relative: bool (true), extraW: pt (18), extraH: pt (18), width, height (absolute), radius: pt (9)}",
            json!({"relative": true, "extraW": 18.0, "extraH": 18.0, "width": 100.0, "height": 100.0, "radius": 9.0}),
        ),
        g(
            "convertToShape.ellipse",
            "Ellipse…",
            SHAPE,
            "{relative: bool (true), extraW: pt (18), extraH: pt (18), width, height (absolute)}",
            json!({"relative": true, "extraW": 18.0, "extraH": 18.0, "width": 100.0, "height": 100.0}),
        ),
        g(
            "distort.freeDistort",
            "Free Distort…",
            DT,
            "{corners: [[x,y]×4] new positions of the bounding-box corners TL, TR, BR, BL in unit box coordinates (identity [[0,0],[1,0],[1,1],[0,1]])}",
            json!({"corners": [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]}),
        ),
        g("distort.puckerBloat", "Pucker & Bloat…", DT, "{amount: % (-200 pucker … 200 bloat, default 0)}", json!({"amount": 0.0})),
        g(
            "distort.roughen",
            "Roughen…",
            DT,
            "{size: 0..100 (5), relative: bool (true: % of size; false: pt), detail: per inch 0..100 (10), points: \"smooth\"|\"corner\", seed: int (0)}",
            json!({"size": 5.0, "relative": true, "detail": 10.0, "points": "smooth", "seed": 0}),
        ),
        g(
            "distort.transform",
            "Transform…",
            DT,
            "{scaleH: % (100), scaleV: % (100), moveH: pt (0), moveV: pt (0), rotate: deg (0), copies: int (0), reflectX: bool, reflectY: bool}",
            json!({"scaleH": 100.0, "scaleV": 100.0, "moveH": 0.0, "moveV": 0.0, "rotate": 0.0, "copies": 0, "reflectX": false, "reflectY": false}),
        ),
        g(
            "distort.tweak",
            "Tweak…",
            DT,
            "{h: amount (10), v: amount (10), relative: bool (true: % of size), anchors: bool (true), in: bool (true), out: bool (true), seed: int (0)}",
            json!({"h": 10.0, "v": 10.0, "relative": true, "anchors": true, "in": true, "out": true, "seed": 0}),
        ),
        g("distort.twist", "Twist…", DT, "{angle: deg (-3600..3600, 10)}", json!({"angle": 10.0})),
        g(
            "distort.zigZag",
            "Zig Zag…",
            DT,
            "{size: (10), relative: bool (false: pt; true: % of size), ridges: per segment 0..100 (4), points: \"smooth\"|\"corner\"}",
            json!({"size": 10.0, "relative": false, "ridges": 4, "points": "smooth"}),
        ),
        g(
            "path.offsetPath",
            "Offset Path…",
            PATH,
            "{offset: pt (10; negative insets), joins: \"miter\"|\"round\"|\"bevel\", miterLimit: (4)}",
            json!({"offset": 10.0, "joins": "miter", "miterLimit": 4.0}),
        ),
        g(
            "path.outlineStroke",
            "Outline Stroke",
            PATH,
            "{width?: pt (defaults to the stroke's weight)} the outline of the stroke the effect is on (on a fill or the object: the top stroke), with its caps, joins, dashes, alignment, width profile and arrowheads",
            json!({}),
        ),
        g("stylize.roundCorners", "Round Corners…", STYLIZE, "{radius: pt (10)}", json!({"radius": 10.0})),
        g(
            "stylize.scribble",
            "Scribble…",
            STYLIZE,
            "{angle: deg (30), overlap: pt (0), strokeWidth: pt (3), curviness: % (5), spacing: pt (5), variation: pt (0.5), seed: int (0)} (simplified: a single hatching scribble outlined to a filled shape)",
            json!({"angle": 30.0, "overlap": 0.0, "strokeWidth": 3.0, "curviness": 5.0, "spacing": 5.0, "variation": 0.5, "seed": 0}),
        ),
        r(
            "stylize.dropShadow",
            "Drop Shadow…",
            STYLIZE,
            "{mode: blend mode (\"multiply\"), opacity: % (75), x: pt (7), y: pt (7), blur: pt (5), color: \"#rrggbb\" (\"#000000\")}",
            json!({"mode": "multiply", "opacity": 75.0, "x": 7.0, "y": 7.0, "blur": 5.0, "color": "#000000"}),
        ),
        r(
            "stylize.innerGlow",
            "Inner Glow…",
            STYLIZE,
            "{mode: blend mode (\"screen\"), opacity: % (75), blur: pt (5), color: \"#rrggbb\" (\"#ffffff\"), source: \"edge\"|\"center\"}",
            json!({"mode": "screen", "opacity": 75.0, "blur": 5.0, "color": "#ffffff", "source": "edge"}),
        ),
        r(
            "stylize.outerGlow",
            "Outer Glow…",
            STYLIZE,
            "{mode: blend mode (\"screen\"), opacity: % (75), blur: pt (5), color: \"#rrggbb\" (\"#ffff00\")}",
            json!({"mode": "screen", "opacity": 75.0, "blur": 5.0, "color": "#ffff00"}),
        ),
        r("stylize.feather", "Feather…", STYLIZE, "{radius: pt (5)}", json!({"radius": 5.0})),
        r("blur.gaussian", "Gaussian Blur…", BLUR, "{radius: pt (5)}", json!({"radius": 5.0})),
    ];
    for (suffix, label) in WARP_STYLES {
        let id: &'static str = match suffix {
            "arc" => "warp.arc",
            "arcLower" => "warp.arcLower",
            "arcUpper" => "warp.arcUpper",
            "arch" => "warp.arch",
            "bulge" => "warp.bulge",
            "shellLower" => "warp.shellLower",
            "shellUpper" => "warp.shellUpper",
            "flag" => "warp.flag",
            "wave" => "warp.wave",
            "fish" => "warp.fish",
            "rise" => "warp.rise",
            "fisheye" => "warp.fisheye",
            "inflate" => "warp.inflate",
            "squeeze" => "warp.squeeze",
            _ => "warp.twist",
        };
        v.push(g(id, label, WARP, WARP_DOC, json!({"bend": 50.0, "horizontal": 0.0, "vertical": 0.0, "orientation": "horizontal"})));
    }
    for (id, label, _) in PATHFINDER_EFFECTS {
        v.push(g(id, label, PATHFINDER, "{} (groups and layers: live Pathfinder over the members)", json!({})));
    }
    v
}

/// The catalogue built once, indexed by id (lookups run per effect per rendered frame).
fn catalog_index() -> &'static std::collections::HashMap<&'static str, EffectInfo> {
    static INDEX: std::sync::OnceLock<std::collections::HashMap<&'static str, EffectInfo>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| effect_catalog().into_iter().map(|e| (e.id, e)).collect())
}

/// Catalogue entry for `id`.
pub fn effect_info(id: &str) -> Option<EffectInfo> {
    catalog_index().get(id).cloned()
}

/// Dialog defaults for `id` (`None` for unknown effects).
pub fn default_params(id: &str) -> Option<Value> {
    catalog_index().get(id).map(|e| e.defaults.clone())
}

/// `params` layered over the defaults of `id` (unknown keys are kept).
pub fn merged_params(id: &str, params: &Value) -> Value {
    let mut out = match catalog_index().get(id).map(|e| &e.defaults) {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    if let Value::Object(p) = params {
        for (k, v) in p {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// A new effect record with default parameters (overridden by `params`).
pub fn new_effect(id: &str, params: &Value) -> Option<Effect> {
    catalog_index().get(id)?;
    Some(Effect { id: id.to_string(), params: merged_params(id, params), visible: true })
}

/// Is `id` a raster (painted) effect?
pub fn is_raster(id: &str) -> bool {
    matches!(id, "stylize.dropShadow" | "stylize.innerGlow" | "stylize.outerGlow" | "stylize.feather" | "blur.gaussian")
}

/// Does `id` change geometry?
pub fn is_geometry(id: &str) -> bool {
    !is_raster(id) && !is_pathfinder(id) && catalog_index().contains_key(id)
}

/// Any visible geometry effect in the list?
pub fn has_geometry(effects: &[Effect]) -> bool {
    effects.iter().any(|e| e.visible && is_geometry(&e.id))
}

/// Context the geometry effects may need from the object's appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeomContext<'a> {
    /// The stroke `path.outlineStroke` outlines (its weight, caps, joins, dashes, alignment,
    /// profile and arrowheads): the stroke the effect sits on, else the object's top painted
    /// stroke. `None` outlines a plain 1 pt stroke.
    pub stroke: Option<&'a StrokeLayer>,
    /// The object's fill rule (inside and outside alignment).
    pub rule: FillRule,
}

impl<'a> GeomContext<'a> {
    /// The context of `n`'s object-level effects.
    pub fn of(n: &'a Node) -> Self {
        let rule = match &n.kind {
            NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
            _ => FillRule::NonZero,
        };
        Self { stroke: n.appearance.stroke().filter(|s| !s.paint.is_none() && s.width > 0.0), rule }
    }

    /// The context of the effects on `item`, one of the same object's appearance items: a
    /// stroke's own effects outline that stroke.
    pub fn item(self, item: &'a AppearanceItem) -> Self {
        match item {
            AppearanceItem::Stroke(s) => Self { stroke: Some(s), ..self },
            AppearanceItem::Fill(_) => self,
        }
    }
}

/// Evaluate the visible geometry effects of `effects` on `path`, in order. `bounds` is the
/// reference box of the first effect (usually the object's geometric bounds); each later effect
/// uses the bounds of its input. Raster and unknown effects are skipped.
pub fn apply_geometry(effects: &[Effect], path: &PathData, bounds: Rect) -> PathData {
    apply_geometry_with(effects, path, bounds, &GeomContext::default())
}

/// [`apply_geometry`] with an explicit [`GeomContext`].
pub fn apply_geometry_with(effects: &[Effect], path: &PathData, bounds: Rect, ctx: &GeomContext) -> PathData {
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
        cur = apply_one(&e.id, &p, &cur, b, ctx);
    }
    cur
}

/// Convenience wrapper on kurbo paths.
pub fn apply_geometry_bez(effects: &[Effect], path: &BezPath, bounds: Rect, ctx: &GeomContext) -> BezPath {
    apply_geometry_with(effects, &PathData::from_bezpath(path), bounds, ctx).to_bezpath()
}

pub(crate) fn apply_one(id: &str, p: &Value, path: &PathData, b: Rect, ctx: &GeomContext) -> PathData {
    use util::*;
    match id {
        "distort.freeDistort" => distort::free_distort(path, b, p),
        "distort.puckerBloat" => distort::pucker_bloat(path, b, num(p, "amount", 0.0)),
        "distort.roughen" => distort::roughen(path, b, p),
        "distort.transform" => distort::transform(path, b, p),
        "distort.tweak" => distort::tweak(path, b, p),
        "distort.twist" => distort::twist(path, b, num(p, "angle", 10.0)),
        "distort.zigZag" => distort::zig_zag(path, b, p),
        "path.offsetPath" => {
            let off = num(p, "offset", 10.0).clamp(-1e5, 1e5);
            if off.abs() < 1e-9 {
                return path.clone();
            }
            vectorcraft_pathops::offset_path(path, off, join(p, "joins"), num(p, "miterLimit", 4.0).clamp(1.0, 500.0))
        }
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
            Some(style) => warp::warp(path, b, style, p),
            None => path.clone(),
        },
    }
}
