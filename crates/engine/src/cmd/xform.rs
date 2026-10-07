//! Commands backing transform/utility tools: free distort, eyedropper colour sampling, artboard move.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::edit::selected_roots;
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.distort",
            "Free Distort",
            [],
            None,
            "{corners: [[x,y]×4] (TL, TR, BR, BL), from?: [x0,y0,x1,y1] (default: selection bounds), ids?} projective warp of anchors and handles",
            has_doc,
            distort
        ),
        cmd!(
            "paint.sampleColor",
            "Sample Color",
            [],
            None,
            "{color, stroke?: bool (default: whichever proxy is active), ids?, stop?: index} sample a colour into the active fill/stroke (and the selection); with `stop`, recolour that stop of the gradient behind the proxy instead (paint.editGradient)",
            has_doc,
            sample_color
        ),
        cmd!(
            "eyedropper.setOptions",
            "Eyedropper Options",
            [],
            None,
            "{sampleSize?: 1|3|5 (pixels square averaged when sampling an image: point, 3×3, 5×5), pickUp?, apply?: {appearance?: {transparency?, fill?: {color?, transparency?, overprint?}, stroke?: {color?, transparency?, overprint?, weight?, cap?, join?, miter?, dash?}}, character?, paragraph?} (bools; a bool for a branch sets all of it)} Eyedropper Options, kept in the preferences: what appearance.copyFrom picks up from the clicked object and applies (only attributes in both trees are copied; every fill and stroke attribute copies the whole appearance stack); {} reads them → the options",
            always,
            eyedropper_options
        ),
        cmd!(
            "artboard.move",
            "Move Artboard",
            [],
            None,
            "{index, dx, dy, moveArt?: bool} move an artboard (and the unlocked art fully inside it)",
            has_doc,
            artboard_move
        ),
    ]
}

// ---------- projective distort ----------

/// A 2D projective transform (homography) mapping the unit square to a quad.
#[derive(Clone, Copy, Debug)]
pub struct Projective {
    m: [f64; 8],
    src: Rect,
}

impl Projective {
    /// Map `src`'s corners (TL, TR, BR, BL) onto `q`.
    pub fn from_rect(src: Rect, q: [Point; 4]) -> Self {
        let [p0, p1, p2, p3] = q;
        let (dx1, dx2, dx3) = (p1.x - p2.x, p3.x - p2.x, p0.x - p1.x + p2.x - p3.x);
        let (dy1, dy2, dy3) = (p1.y - p2.y, p3.y - p2.y, p0.y - p1.y + p2.y - p3.y);
        let (g, h) = if dx3.abs() < 1e-12 && dy3.abs() < 1e-12 {
            (0.0, 0.0)
        } else {
            let den = dx1 * dy2 - dx2 * dy1;
            if den.abs() < 1e-12 { (0.0, 0.0) } else { ((dx3 * dy2 - dx2 * dy3) / den, (dx1 * dy3 - dx3 * dy1) / den) }
        };
        let a = p1.x - p0.x + g * p1.x;
        let b = p3.x - p0.x + h * p3.x;
        let d = p1.y - p0.y + g * p1.y;
        let e = p3.y - p0.y + h * p3.y;
        Self { m: [a, b, p0.x, d, e, p0.y, g, h], src }
    }

    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f, g, h] = self.m;
        let u = if self.src.width().abs() > 1e-12 { (p.x - self.src.x0) / self.src.width() } else { 0.0 };
        let v = if self.src.height().abs() > 1e-12 { (p.y - self.src.y0) / self.src.height() } else { 0.0 };
        let w = g * u + h * v + 1.0;
        let w = if w.abs() < 1e-9 { 1e-9_f64.copysign(w) } else { w };
        Point::new((a * u + b * v + c) / w, (d * u + e * v + f) / w)
    }

    /// Affine approximation at a point (for objects that can't be warped: text, images, symbols).
    fn affine_near(&self, p: Point) -> Affine {
        let eps = 1.0;
        let o = self.apply(p);
        let ex = self.apply(p + Vec2::new(eps, 0.0)) - o;
        let ey = self.apply(p + Vec2::new(0.0, eps)) - o;
        let lin = Affine::new([ex.x / eps, ex.y / eps, ey.x / eps, ey.y / eps, 0.0, 0.0]);
        Affine::translate(o.to_vec2()) * lin * Affine::translate(-p.to_vec2())
    }
}

/// Warp `n`'s anchors and handles; gradients (pinned first if unplaced) follow the warp's affine
/// approximation at their centre.
fn warp_node(n: &mut Node, pr: &Projective) {
    n.pin_gradients();
    match &mut n.kind {
        NodeKind::Path { path, live, .. } => {
            *live = None;
            for sp in &mut path.subpaths {
                for a in &mut sp.anchors {
                    a.p = pr.apply(a.p);
                    a.h_in = pr.apply(a.h_in);
                    a.h_out = pr.apply(a.h_out);
                }
            }
        }
        NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } | NodeKind::Compound { children, .. } => {
            for c in children.iter_mut() {
                warp_node(Arc::make_mut(c), pr);
            }
        }
        _ => {
            // No editable points: the affine approximation (which maps the gradients too).
            if let Some(b) = n.geometric_bounds() {
                n.transform(pr.affine_near(b.center()), false);
            }
            return;
        }
    }
    n.appearance.warp_gradients(&|p| pr.affine_near(p));
}

fn distort(s: &mut Session, p: &Value) -> Result<Value> {
    let corners: Vec<Point> = p
        .get("corners")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| Some(Point::new(c.get(0)?.as_f64()?, c.get(1)?.as_f64()?))).collect())
        .unwrap_or_default();
    if corners.len() != 4 {
        return Err(bad("object.distort", "corners must be 4 [x,y] points (TL, TR, BR, BL)"));
    }
    let ids = match ids_param(p, "ids") {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    let src = match p.get("from").and_then(Value::as_array) {
        Some(a) if a.len() == 4 => {
            let f: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
            if f.len() != 4 {
                return Err(bad("object.distort", "from must be [x0,y0,x1,y1]"));
            }
            Rect::new(f[0], f[1], f[2], f[3])
        }
        _ => s.doc()?.doc.bounds_of(&ids, false).ok_or_else(|| EngineError::Other("nothing to distort".into()))?,
    };
    if src.width().abs() < 1e-9 || src.height().abs() < 1e-9 {
        return Err(EngineError::Other("cannot distort a zero-size bounding box".into()));
    }
    let pr = Projective::from_rect(src, [corners[0], corners[1], corners[2], corners[3]]);
    s.edit("Free Distort", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                warp_node(n, &pr);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

// ---------- eyedropper ----------

/// The focal (topmost) fill's attributes the Eyedropper picks up or applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FillAttrs {
    pub color: bool,
    /// Its own opacity and blend mode.
    pub transparency: bool,
    pub overprint: bool,
}

impl FillAttrs {
    pub const ALL: Self = Self { color: true, transparency: true, overprint: true };
}

impl Default for FillAttrs {
    fn default() -> Self {
        Self::ALL
    }
}

/// The focal (topmost) stroke's attributes the Eyedropper picks up or applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StrokeAttrs {
    pub color: bool,
    /// Its own opacity and blend mode.
    pub transparency: bool,
    pub overprint: bool,
    pub weight: bool,
    pub cap: bool,
    pub join: bool,
    /// The miter limit.
    pub miter: bool,
    /// The dash pattern.
    pub dash: bool,
}

impl StrokeAttrs {
    pub const ALL: Self = Self { color: true, transparency: true, overprint: true, weight: true, cap: true, join: true, miter: true, dash: true };
}

impl Default for StrokeAttrs {
    fn default() -> Self {
        Self::ALL
    }
}

/// Appearance attributes: the object's transparency (opacity and blend mode) and its focal fill and
/// stroke. With every fill and stroke attribute, the whole appearance stack (every fill and stroke,
/// with their effects, and the object's effects) is copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppearanceAttrs {
    pub transparency: bool,
    pub fill: FillAttrs,
    pub stroke: StrokeAttrs,
}

impl Default for AppearanceAttrs {
    fn default() -> Self {
        Self { transparency: true, fill: FillAttrs::ALL, stroke: StrokeAttrs::ALL }
    }
}

/// One of the Eyedropper Options' trees: what it picks up, or what it applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EyedropperAttrs {
    pub appearance: AppearanceAttrs,
    /// Type's character attributes (font, size, leading, tracking…; its paints follow `appearance`).
    pub character: bool,
    /// Type's paragraph attributes.
    pub paragraph: bool,
}

impl Default for EyedropperAttrs {
    fn default() -> Self {
        Self { appearance: Default::default(), character: true, paragraph: true }
    }
}

impl EyedropperAttrs {
    /// No attribute at all.
    pub fn is_empty(&self) -> bool {
        fn any(v: &Value) -> bool {
            match v {
                Value::Bool(b) => *b,
                Value::Object(o) => o.values().any(any),
                _ => false,
            }
        }
        !any(&json!(self))
    }
}

/// Eyedropper Options (stored in the preferences): the raster sample size and what the Eyedropper
/// picks up from the object it clicks and applies to the selection (`appearance.copyFrom`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EyedropperOptions {
    /// Pixels square averaged when sampling an image: 1 (point), 3 or 5.
    pub sample_size: u32,
    pub pick_up: EyedropperAttrs,
    pub apply: EyedropperAttrs,
}

impl Default for EyedropperOptions {
    fn default() -> Self {
        Self { sample_size: 1, pick_up: Default::default(), apply: Default::default() }
    }
}

impl EyedropperOptions {
    /// These options with `sampleSize`, `pickUp` and `apply` from `p` applied: a tree given as an
    /// object sets the flags it names, a bool sets every flag of its branch.
    pub(crate) fn merged(&self, p: &Value) -> std::result::Result<Self, String> {
        let mut v = serde_json::to_value(self).map_err(|e| e.to_string())?;
        if let Some(n) = p.get("sampleSize").filter(|n| !n.is_null()) {
            let n = n.as_u64().filter(|n| matches!(n, 1 | 3 | 5)).ok_or("`sampleSize` must be 1, 3 or 5")?;
            v["sampleSize"] = json!(n);
        }
        for key in ["pickUp", "apply"] {
            if let Some(t) = p.get(key).filter(|t| !t.is_null()) {
                merge_flags(&mut v[key], t, key)?;
            }
        }
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    /// What one copy takes: the attributes both picked up and applied, with the `pickUp` / `apply`
    /// params of that call merged over the options.
    pub(crate) fn attrs(&self, p: &Value) -> std::result::Result<EyedropperAttrs, String> {
        let o = self.merged(p)?;
        let mut both = json!(o.pick_up);
        and_flags(&mut both, &json!(o.apply));
        serde_json::from_value(both).map_err(|e| e.to_string())
    }
}

/// Set the flags of `dst` named by `src` (`path`: where `dst` is, for errors).
fn merge_flags(dst: &mut Value, src: &Value, path: &str) -> std::result::Result<(), String> {
    match (dst, src) {
        (Value::Bool(d), Value::Bool(b)) => {
            *d = *b;
            Ok(())
        }
        (Value::Object(d), Value::Bool(_)) => d.values_mut().try_for_each(|v| merge_flags(v, src, path)),
        (Value::Object(d), Value::Object(s)) => s.iter().try_for_each(|(k, v)| match d.get_mut(k) {
            Some(dv) => merge_flags(dv, v, &format!("{path}.{k}")),
            None => Err(format!("unknown option `{path}.{k}`")),
        }),
        _ => Err(format!("`{path}` must be a bool or an object of bools")),
    }
}

/// `dst` and `other` flag by flag.
fn and_flags(dst: &mut Value, other: &Value) {
    match (dst, other) {
        (Value::Bool(d), Value::Bool(b)) => *d &= *b,
        (Value::Object(d), Value::Object(o)) => d.iter_mut().for_each(|(k, v)| and_flags(v, &o[k])),
        _ => {}
    }
}

fn eyedropper_options(s: &mut Session, p: &Value) -> Result<Value> {
    s.prefs.eyedropper = s.prefs.eyedropper.merged(p).map_err(|e| bad("eyedropper.setOptions", e))?;
    Ok(json!(s.prefs.eyedropper))
}

fn sample_color(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.sampleColor";
    let stroke = p.get("stroke").and_then(Value::as_bool).unwrap_or(!s.fill_active);
    let c = p.get("color").ok_or_else(|| bad(C, "missing color"))?;
    let (id, mut q) = match p.get("stop") {
        None | Some(Value::Null) => (if stroke { "paint.setStroke" } else { "paint.setFill" }, json!({ "color": c })),
        Some(v) => {
            let i = v.as_u64().ok_or_else(|| bad(C, "`stop` must be a stop index"))? as usize;
            let color = color_value(c).ok_or_else(|| bad(C, format!("bad color {c}")))?;
            let (fill, stroke_paint) = s.proxy_paints();
            let Paint::Gradient(g) = (if stroke { stroke_paint } else { fill }) else {
                return Err(bad(C, "the paint behind the proxy is not a gradient"));
            };
            let mut stops = g.gradient.stops;
            let n = stops.len();
            stops.get_mut(i).ok_or_else(|| bad(C, format!("no stop {i} (the gradient has {n})")))?.set_color(color, None);
            ("paint.editGradient", json!({ "stops": vectorcraft_tools::params::stops_json(&stops), "stroke": stroke }))
        }
    };
    if let Some(ids) = p.get("ids") {
        q["ids"] = ids.clone();
    }
    let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.into()))?;
    (spec.run)(s, &q)
}

// ---------- artboards ----------

fn artboard_move(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("artboard.move", "missing index"))? as usize;
    let dv = Vec2::new(f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    let move_art = bool_or(p, "moveArt", false);
    let st = s.doc()?;
    let ab = st.doc.artboards.get(i).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
    if ab.locked {
        return Err(EngineError::Other(format!("artboard “{}” is locked", ab.name)));
    }
    let rect = ab.rect;
    // Top-level objects (children of layers) lying entirely inside the artboard.
    let mut art = vec![];
    if move_art {
        fn collect(n: &Node, rect: Rect, out: &mut Vec<NodeId>) {
            for c in n.children().into_iter().flatten() {
                if c.locked {
                    continue;
                }
                if c.is_layer() {
                    collect(c, rect, out);
                } else if let Some(b) = c.geometric_bounds()
                    && rect.contains(Point::new(b.x0, b.y0))
                    && rect.contains(Point::new(b.x1, b.y1))
                {
                    out.push(c.id);
                }
            }
        }
        for l in &st.doc.layers {
            if !l.locked {
                collect(l, rect, &mut art);
            }
        }
    }
    let scale_strokes = s.prefs.scale_strokes;
    s.edit("Move Artboard", |d, _| {
        let a = d.artboards.get_mut(i).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
        a.rect = a.rect + dv;
        for id in &art {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(dv), scale_strokes);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "index": i, "moved": art.iter().map(|i| i.0).collect::<Vec<_>>() }))
}
