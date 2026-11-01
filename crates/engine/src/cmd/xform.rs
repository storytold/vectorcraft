//! Commands backing transform/utility tools: free distort, eyedropper colour sampling, artboard move.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Node, NodeKind, Scaling};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::edit::{outermost, selected_roots};
use super::{
    CommandSpec, NodeId, Result, Session, always, bad, bool_or, check_numbers, check_switches, checked_ids_param, cmd, color_value, f64_or,
    find_command, finite_numbers, has_doc, index,
};
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
            "{color, stroke?: bool (default: whichever proxy is active), ids?, stop?: index, clearStroke?: bool} sample a color into the active fill/stroke (and the selection; `ids: []` changes only the paint default); `clearStroke: true` requires `stroke: false`, `ids: []` and no `stop`, and clears the default stroke after sampling an image into the fill; with `stop`, recolor that stop of the gradient behind the proxy instead (paint.editGradient); in a CMYK document an RGB color is stored as CMYK through the color settings, as paint.setFill stores it",
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
            "{index | indices: [index…], dx, dy, moveArt?: bool, copy?: bool, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} move artboards (and the art fully inside them; locked and hidden art, and art also on another locked artboard, only with prefs moveLockedWithArtboard), several in one step (an error when one is locked); `copy` (Alt-drag) leaves them and moves copies, which start unlocked → {index (the first), indices: the artboards moved or the copies, moved: the art moved or the copies}",
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
    const C: &str = "object.distort";
    // All four corners or nothing, each exactly [x, y]: nothing malformed is left out.
    let corners: Option<Vec<Point>> = p
        .get("corners")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 4)
        .and_then(|a| a.iter().map(|c| finite_numbers(c).map(|[x, y]| Point::new(x, y))).collect());
    let Some(&[tl, tr, br, bl]) = corners.as_deref() else { return Err(bad(C, "corners must be 4 [x, y] points (TL, TR, BR, BL)")) };
    let ids =
        if p.get("ids").is_some_and(|v| !v.is_null()) { outermost(&s.doc()?.doc, checked_ids_param(s, p, "ids", C)?) } else { selected_roots(s)? };
    if ids.is_empty() {
        return Err(bad(C, "nothing to distort"));
    }
    let src = match p.get("from").filter(|v| !v.is_null()) {
        None => s.doc()?.doc.bounds_of(&ids, false).ok_or_else(|| bad(C, "nothing to distort"))?,
        Some(v) => {
            let [x0, y0, x1, y1] = finite_numbers(v).ok_or_else(|| bad(C, "from must be [x0, y0, x1, y1]"))?;
            Rect::new(x0, y0, x1, y1)
        }
    };
    if !src.width().is_finite() || !src.height().is_finite() || src.width().abs() < 1e-9 || src.height().abs() < 1e-9 {
        return Err(bad(C, "cannot distort a zero-size or non-finite bounding box"));
    }
    let pr = Projective::from_rect(src, [tl, tr, br, bl]);
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
    let stroke = super::switch_or(p, "stroke", !s.fill_active, C)?;
    let clear_stroke = super::switch_or(p, "clearStroke", false, C)?;
    if clear_stroke && (stroke || p.get("stroke") != Some(&Value::Bool(false)) || p.get("ids") != Some(&json!([])) || p.get("stop").is_some()) {
        return Err(bad(C, "`clearStroke` requires `stroke: false`, `ids: []` and no `stop`"));
    }
    let c = p.get("color").ok_or_else(|| bad(C, "missing color"))?;
    let (id, mut q) = match p.get("stop") {
        None | Some(Value::Null) => (if stroke { "paint.setStroke" } else { "paint.setFill" }, json!({ "color": c })),
        Some(v) => {
            let i = v.as_u64().map(index).ok_or_else(|| bad(C, "`stop` must be a stop index"))?;
            let color = super::paint::new_color(s, p, color_value(c).ok_or_else(|| bad(C, format!("bad color {c}")))?);
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
    let result = (spec.run)(s, &q)?;
    if clear_stroke {
        s.paint.stroke = Paint::None;
    }
    Ok(result)
}

// ---------- artboards ----------

fn artboard_move(s: &mut Session, p: &Value) -> Result<Value> {
    let indices = super::panelcmds::artboard_indices(&s.doc()?.doc, p, "artboard.move")?;
    check_numbers(p, &["dx", "dy"], "artboard.move")?;
    check_switches(p, &["moveArt", "copy", "patterns"], "artboard.move")?;
    let dv = Vec2::new(f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    let move_art = bool_or(p, "moveArt", false);
    let copy = bool_or(p, "copy", false);
    if !copy {
        super::panelcmds::refuse_locked(&s.doc()?.doc, &indices)?;
    }
    // Each artboard's art, an object counted once (artboards can overlap).
    let mut arts: Vec<(usize, Vec<NodeId>)> = vec![];
    for &i in &indices {
        let art = super::panelcmds::artboard_art(s, i, move_art, !copy)?;
        let art = art.into_iter().filter(|id| !arts.iter().any(|(_, a)| a.contains(id))).collect();
        arts.push((i, art));
    }
    let ids: Vec<NodeId> = arts.iter().flat_map(|(_, a)| a.iter().copied()).collect();
    let sc = Scaling { patterns: super::object::transform_patterns(s, p, &ids)?, ..Scaling::default() };
    let (index, moved) = s.edit(if copy { "Duplicate Artboard" } else { "Move Artboard" }, |d, _| {
        let (mut index, mut moved) = (vec![], vec![]);
        for (i, art) in &arts {
            // A copy keeps the artboard (and its art) where they are and moves duplicates instead.
            if copy {
                let (at, copies) = super::panelcmds::copy_artboard(d, *i, dv, art, sc)?;
                index.push(at);
                moved.extend(copies);
                continue;
            }
            let a = d.artboards.get_mut(*i).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
            a.rect = a.rect + dv;
            let id = a.id;
            d.move_artboard_guides(id, dv);
            for id in art {
                if let Some(n) = d.node_mut(*id) {
                    n.transform(Affine::translate(dv), sc);
                }
            }
            index.push(*i);
            moved.extend(art.iter().copied());
        }
        Ok((index, moved))
    })?;
    Ok(json!({ "index": index.first(), "indices": index, "moved": moved.iter().map(|i| i.0).collect::<Vec<_>>() }))
}
