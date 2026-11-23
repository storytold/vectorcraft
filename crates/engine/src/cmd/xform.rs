//! Commands backing transform/utility tools: free distort, eyedropper colour sampling, artboard move.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Node, NodeKind, OrientedBox, Scaling};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::edit::{outermost, selected_roots};
use super::{
    CommandSpec, NodeId, Result, Session, always, bad, bool_or, check_numbers, check_switches, checked_ids_param, cmd, color_value, f64_or, f64_req,
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
            "{corners: [[x,y]×4] (TL, TR, BR, BL), from?: [x0,y0,x1,y1] (default: selection bounds), angle?: degrees (counter-clockwise; `from`, or the selection bounds, is then a box in the frame turned by it, as a rotated bounding box is; default 0), ids?} projective warp of anchors and handles; an image's four corners go exactly where the warp takes them and its pixels are drawn in perspective between them (an error, and nothing changes, when an image's corners would fold over, fall on one line, cross the warp's horizon or lie too close together for a perspective map); a linked opacity mask's art is warped with its object, an unlinked one stays; each warped object's bounding box (the objects inside groups and the art of linked masks too) turns as the warp turns the art at its center (object.resetBoundingBox squares it again); corners where the box already stands change nothing and add no undo step",
            has_doc,
            distort
        ),
        cmd!(
            "paint.sampleColor",
            "Sample Color",
            [],
            None,
            "{color (#rrggbbaa and #rgba are refused), stroke?: bool (default: whichever proxy is active), ids?, stop?: index, clearStroke?: bool} sample a color into the active fill/stroke (and the selection; `ids: []` changes only the paint default, and so does a sample without ids while the Live Paint Bucket is the active tool, `stop` included); `clearStroke: true` requires `stroke: false`, `ids: []` and no `stop`, and clears the default stroke after sampling an image into the fill; with `stop`, recolor that stop of the gradient behind the proxy instead (paint.editGradient); with `stop` and objects in `ids`, whatever tool is active, the stops come from the topmost fill (or stroke) of the first object that paint.editGradient edits for those ids (a compound path for its member); in a CMYK document an RGB color is stored as CMYK through the color settings, as paint.setFill stores it",
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
            "{index | indices: [index…], dx, dy, moveArt?: bool, copy?: bool, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), lockedAndHidden?: bool (default: prefs moveLockedWithArtboard)} move artboards (and the art fully inside them; locked and hidden art, and art also on another locked artboard, only with lockedAndHidden), several in one step (an error when one is locked); `copy` (Alt-drag) leaves them and moves copies, which start unlocked → {index (the first), indices: the artboards moved or the copies, moved: the art moved or the copies}",
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
    /// Page → the frame `src` is measured in.
    pre: Affine,
}

impl Projective {
    /// Map the corners (TL, TR, BR, BL) of `src`, a box that may be turned, onto the page points `q`.
    pub fn from_box(bx: OrientedBox, q: [Point; 4]) -> Self {
        let src = bx.rect;
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
        Self { m: [a, b, p0.x, d, e, p0.y, g, h], src, pre: bx.to_doc().inverse() }
    }

    /// Whether `p` maps in front of the warp's horizon, on the side the box's own corners are.
    fn in_front(&self, p: Point) -> bool {
        let [.., g, h] = self.m;
        let p = self.pre * p;
        let u = if self.src.width().abs() > 1e-12 { (p.x - self.src.x0) / self.src.width() } else { 0.0 };
        let v = if self.src.height().abs() > 1e-12 { (p.y - self.src.y0) / self.src.height() } else { 0.0 };
        g * u + h * v + 1.0 > 1e-9
    }

    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f, g, h] = self.m;
        let p = self.pre * p;
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
/// approximation at their center. The art of a linked opacity mask is warped with the object it
/// masks; an unlinked one stays where it is. Each warped object's bounding-box angle follows the
/// warp's affine approximation at that object's own center before the warp (as for the objects
/// with no editable points); an object with no bounds keeps its angle, and a layer has none.
/// Returns `n`'s geometric bounds before the warp ([`Node::geometric_bounds`]; none for a layer),
/// gathered from its children's so that each subtree is measured once.
fn warp_node(n: &mut Node, pr: &Projective) -> Option<Rect> {
    n.pin_gradients();
    // The children before these are left out of a group's or compound path's bounds.
    let skip = n.children().map_or(0, Vec::len) - n.editing_children().len();
    let before = match &mut n.kind {
        NodeKind::Path { path, live, .. } => {
            let b = path.bounds();
            *live = None;
            for sp in &mut path.subpaths {
                for a in &mut sp.anchors {
                    a.p = pr.apply(a.p);
                    a.h_in = pr.apply(a.h_in);
                    a.h_out = pr.apply(a.h_out);
                }
            }
            b
        }
        NodeKind::Layer { children, .. } => {
            warp_children(children, pr, |_| false);
            None
        }
        // A clip group is bounded by its clip path.
        NodeKind::Group { children, clip: true } => warp_children(children, pr, |i| i == 0),
        NodeKind::Group { children, .. } | NodeKind::Compound { children, .. } => warp_children(children, pr, |i| i >= skip),
        // An image's corners follow the warp exactly ([`Node::set_image_corners`]); [`distort`]
        // has checked that they stay convex.
        NodeKind::Image(_) => {
            let b = n.geometric_bounds();
            if let Some(q) = n.image_corners() {
                let _ = n.set_image_corners(q.map(|p| pr.apply(p)));
            }
            b
        }
        _ => {
            // No editable points: the affine approximation (which maps the gradients too).
            let b = n.geometric_bounds();
            if let Some(b) = b {
                n.transform(pr.affine_near(b.center()), false);
            }
            return b;
        }
    };
    if let Some(b) = before {
        n.bbox_angle = vectorcraft_doc::orient::transformed_angle(n.bbox_angle, pr.affine_near(b.center()));
    }
    if let Some(m) = n.extras_if_any().and_then(|x| x.mask.as_mut()).filter(|m| m.linked) {
        warp_node(Arc::make_mut(&mut m.art), pr);
    }
    n.appearance.warp_gradients(&|p| pr.affine_near(p));
    before
}

/// Whether the warp from `src` to `q` ([`Projective::from_box`]) takes the corners of `src` to `q`,
/// within a millionth of `q`'s shortest side plus the rounding error of coordinates as far from `q`'s
/// first corner as the others. The warp is solved and measured with `src` and `q` moved to the
/// origin, so coordinates far from it do not widen the allowance.
fn reaches_corners(src: OrientedBox, q: [Point; 4]) -> bool {
    let origin = q[0].to_vec2();
    let q = q.map(|p| p - origin);
    let src = OrientedBox { rect: src.rect - src.rect.origin().to_vec2(), ..src };
    let pr = Projective::from_box(src, q);
    let side = (0..4).map(|i| q[i].distance(q[(i + 1) % 4])).fold(f64::INFINITY, f64::min);
    let far = q.iter().map(|p| p.x.abs().max(p.y.abs())).fold(0.0, f64::max);
    let tol = side * 1e-6 + far * 1e-12;
    side.is_finite() && side > 0.0 && src.corners().iter().zip(q).all(|(c, want)| pr.apply(*c).distance(want) <= tol)
}

/// Warps each of `children` and returns the union of the pre-warp bounds of those `counts` picks
/// by index.
fn warp_children(children: &mut [Arc<Node>], pr: &Projective, counts: impl Fn(usize) -> bool) -> Option<Rect> {
    children.iter_mut().enumerate().fold(None, |acc, (i, c)| {
        let b = warp_node(Arc::make_mut(c), pr);
        if counts(i) { vectorcraft_geom::union_opt(acc, b) } else { acc }
    })
}

/// The first image in `n` (its children and the art of linked opacity masks, as [`warp_node`]
/// reaches them) whose corners `pr` would fold, collapse or carry past the warp's horizon, with
/// why.
fn unwarpable_image(n: &Node, pr: &Projective) -> Option<(vectorcraft_doc::NodeId, Unwarpable)> {
    if let (Some(q), NodeKind::Image(im)) = (n.image_corners(), &n.kind) {
        if !q.iter().all(|p| pr.in_front(*p)) {
            return Some((n.id, Unwarpable::Horizon));
        }
        if let Err(e) = vectorcraft_doc::image_quad::check_for(im, &q.map(|p| pr.apply(p))) {
            return Some((
                n.id,
                match e {
                    vectorcraft_doc::image_quad::QuadError::NotFinite | vectorcraft_doc::image_quad::QuadError::NotConvex => Unwarpable::NotConvex,
                    vectorcraft_doc::image_quad::QuadError::TooNarrow => Unwarpable::TooNarrow,
                },
            ));
        }
    }
    warped_parts(n).into_iter().find_map(|c| unwarpable_image(c, pr))
}

/// The first image in `n` that [`warp_node`] reaches (its children and the art of linked opacity
/// masks).
fn first_image(n: &Node) -> Option<vectorcraft_doc::NodeId> {
    if matches!(n.kind, NodeKind::Image(_)) {
        return Some(n.id);
    }
    warped_parts(n).into_iter().find_map(first_image)
}

/// The objects inside `n` that [`warp_node`] warps point by point: the children of layers, groups
/// and compound paths, and the art of a linked opacity mask on those, paths and images. Other
/// objects (blends, envelopes, repeats and the rest) take the warp's affine approximation whole.
fn warped_parts(n: &Node) -> Vec<&Node> {
    let inside = matches!(n.kind, NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Compound { .. });
    let mask_too = inside || matches!(n.kind, NodeKind::Path { .. } | NodeKind::Image(_));
    let children = n.children().filter(|_| inside).into_iter().flatten().map(|c| &**c);
    let mask = n.extras().mask.as_deref().filter(|m| mask_too && m.linked).map(|m| &*m.art);
    children.chain(mask).collect()
}

/// The error of `command` on image `id`, which is distorted (Perspective Distort or Free Distort)
/// in a way the command cannot keep.
pub(crate) fn distorted_image_error(command: &str, id: vectorcraft_doc::NodeId) -> EngineError {
    bad(command, format!("object {} is an image changed by Perspective Distort or Free Distort, which this command does not work on", id.0))
}

/// Why an image cannot follow the corners of a distort.
#[derive(Clone, Copy)]
enum Unwarpable {
    /// Its corners would fold over, meet or fall on one line.
    NotConvex,
    /// A corner would cross the warp's horizon.
    Horizon,
    /// Its corners would be too close together for a perspective map to place them.
    TooNarrow,
}

/// The error of `object.distort` when image `id` cannot follow its corners.
fn unwarpable_error(id: vectorcraft_doc::NodeId, why: Unwarpable) -> EngineError {
    const C: &str = "object.distort";
    match why {
        Unwarpable::NotConvex => bad(C, format!("image {} cannot be distorted this way: its corners would not make a convex shape", id.0)),
        Unwarpable::Horizon => bad(C, format!("image {} cannot be distorted this way: its corners would cross the warp's horizon", id.0)),
        Unwarpable::TooNarrow => {
            bad(C, format!("image {} cannot be distorted this way: its corners would be too close together for a perspective map", id.0))
        }
    }
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
    let angle = p.get("angle").filter(|v| !v.is_null()).map(|_| f64_req(p, "angle", C)).transpose()?.map_or(0.0, vectorcraft_geom::normalize_deg);
    let src = match p.get("from").filter(|v| !v.is_null()) {
        None if angle == 0.0 => OrientedBox::aligned(s.doc()?.doc.bounds_of(&ids, false).ok_or_else(|| bad(C, "nothing to distort"))?),
        None => {
            // The selection's bounds square to the turned frame.
            let doc = &s.doc()?.doc;
            let (symbols, to_local) = (doc.symbol_boxes(), OrientedBox { angle, rect: Rect::ZERO }.to_doc().inverse());
            let rect = ids
                .iter()
                .filter_map(|id| doc.node(*id))
                .fold(None, |acc, n| vectorcraft_geom::union_opt(acc, n.bounds_in(&symbols, to_local, false)));
            OrientedBox { angle, rect: rect.ok_or_else(|| bad(C, "nothing to distort"))? }
        }
        Some(v) => {
            let [x0, y0, x1, y1] = finite_numbers(v).ok_or_else(|| bad(C, "from must be [x0, y0, x1, y1]"))?;
            OrientedBox { angle, rect: Rect::new(x0, y0, x1, y1) }
        }
    };
    let r = src.rect;
    if !r.width().is_finite() || !r.height().is_finite() || r.width().abs() < 1e-9 || r.height().abs() < 1e-9 {
        return Err(bad(C, "cannot distort a zero-size or non-finite bounding box"));
    }
    let ids_json = json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() });
    // Corners that stay where they are change nothing: no edit and no undo step.
    if [tl, tr, br, bl] == src.corners() {
        return Ok(ids_json);
    }
    let pr = Projective::from_box(src, [tl, tr, br, bl]);
    // An image's corners follow the handles exactly, so they must stay a convex shape: nothing
    // changes when one would not.
    let doc = &s.doc()?.doc;
    // Corners that do not make a convex shape give no perspective map (the warp falls back to an
    // affine one), so an image cannot follow them.
    if vectorcraft_doc::image_quad::check(&[tl, tr, br, bl]).is_err()
        && let Some(id) = ids.iter().filter_map(|id| doc.node(*id)).find_map(first_image)
    {
        return Err(unwarpable_error(id, Unwarpable::NotConvex));
    }
    // Corners so close together that the map falls back to an affine one: the box's corners do
    // not reach the handles, so an image cannot follow them either.
    if !reaches_corners(src, [tl, tr, br, bl])
        && let Some(id) = ids.iter().filter_map(|id| doc.node(*id)).find_map(first_image)
    {
        return Err(unwarpable_error(id, Unwarpable::TooNarrow));
    }
    if let Some((id, why)) = ids.iter().filter_map(|id| doc.node(*id)).find_map(|n| unwarpable_image(n, &pr)) {
        return Err(unwarpable_error(id, why));
    }
    s.edit("Free Distort", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                warp_node(n, &pr);
            }
        }
        Ok(())
    })?;
    Ok(ids_json)
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
    pub const NONE: Self = Self { color: false, transparency: false, overprint: false };
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
    pub const NONE: Self =
        Self { color: false, transparency: false, overprint: false, weight: false, cap: false, join: false, miter: false, dash: false };
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
            let (paint, source) = match given_stop_paint(s, p, stroke)? {
                Some(paint) => (paint, "the paint of the first object in `ids`"),
                None => {
                    let (fill, stroke_paint) = s.proxy_paints();
                    (if stroke { stroke_paint } else { fill }, "the paint behind the proxy")
                }
            };
            let Paint::Gradient(g) = paint else {
                return Err(bad(C, format!("{source} is not a gradient")));
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
    let result = super::run_checked(s, spec, &q)?;
    if clear_stroke {
        s.paint.stroke = Paint::None;
    }
    Ok(result)
}

/// The paint whose gradient `paint.sampleColor` recolors a stop of when `p` gives `stop` and
/// objects in `ids`: the fill (or stroke, `stroke`) of the first object `paint.editGradient` edits
/// for those ids (a compound path for its member, a group's painted objects, not the art of a graph
/// design in a series). None without `stop` or without objects in `ids`: the gradient behind the
/// proxy is used.
fn given_stop_paint(s: &Session, p: &Value, stroke: bool) -> Result<Option<Paint>> {
    const C: &str = "paint.sampleColor";
    if p.get("stop").is_none_or(Value::is_null) || p.get("ids").and_then(Value::as_array).is_none_or(Vec::is_empty) {
        return Ok(None);
    }
    let q = json!({ "ids": p.get("ids"), "stroke": stroke });
    let item = super::appearance::item_target(s, &q, C)?.of_kind(s, !stroke);
    let ids = super::appearance::drop_design_art_rows(s, item.targets_for(s, &q, !stroke)?, item, !stroke, C)?;
    let d = &s.doc()?.doc;
    let Some(n) = ids.first().and_then(|id| d.node(*id)) else { return Ok(None) };
    Ok(Some(super::paint::proxy_paint(n, stroke, None)))
}

// ---------- artboards ----------

fn artboard_move(s: &mut Session, p: &Value) -> Result<Value> {
    let indices = super::panelcmds::artboard_indices(&s.doc()?.doc, p, "artboard.move")?;
    check_numbers(p, &["dx", "dy"], "artboard.move")?;
    check_switches(p, &["moveArt", "copy", "patterns"], "artboard.move")?;
    let dv = Vec2::new(f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    let move_art = bool_or(p, "moveArt", false);
    let copy = bool_or(p, "copy", false);
    let locked_art = super::panelcmds::locked_and_hidden(s, p, move_art, "artboard.move")?;
    if !copy {
        super::panelcmds::refuse_locked(&s.doc()?.doc, &indices)?;
    }
    // Each artboard's art, an object counted once (artboards can overlap).
    let mut arts: Vec<(usize, Vec<NodeId>)> = vec![];
    for &i in &indices {
        let art = super::panelcmds::artboard_art(s, i, move_art, locked_art, !copy)?;
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
                    super::object::keep_pixel_aligned(n, sc);
                }
            }
            index.push(*i);
            moved.extend(art.iter().copied());
        }
        Ok((index, moved))
    })?;
    // Transform Again repeats it on the artboards it left: the moved ones, or the copies.
    let mut again = p.clone();
    if let Some(o) = again.as_object_mut() {
        o.remove("index");
        o.remove("indices");
        // The values this move used, not the preferences of the moment the repeat runs.
        o.insert("moveArt".into(), json!(move_art));
        o.insert("copy".into(), json!(copy));
        o.insert("patterns".into(), json!(sc.patterns));
        o.insert("lockedAndHidden".into(), json!(locked_art));
        let d = &s.doc()?.doc;
        o.insert("ids".into(), json!(index.iter().filter_map(|i| d.artboards.get(*i).map(|a| a.id)).collect::<Vec<_>>()));
    }
    let st = s.doc_mut()?;
    match &mut st.interaction {
        Some(it) => it.artboards_again = Some(again),
        None => {
            st.last_transform = Some(crate::LastTransform::Artboards(again));
            st.last_perspective = None;
        }
    }
    Ok(json!({ "index": index.first(), "indices": index, "moved": moved.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// Transform Again after an artboard move or copy: the same move again, on the artboards the last
/// one left (by id, so a copy repeated copies the copies). They are gone: an error, not a move of
/// whatever now has their positions.
pub(crate) fn artboard_move_again(s: &mut Session, again: &Value) -> Result<Value> {
    let ids: Vec<u32> =
        again.get("ids").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).filter_map(|v| u32::try_from(v).ok()).collect();
    let indices: Vec<usize> = {
        let d = &s.doc()?.doc;
        ids.iter().filter_map(|id| d.artboards.iter().position(|a| a.id == *id)).collect()
    };
    if indices.is_empty() {
        return Err(EngineError::Other("the artboards Transform Again would move are gone".into()));
    }
    let mut p = again.clone();
    if let Some(o) = p.as_object_mut() {
        o.remove("ids");
        o.insert("indices".into(), json!(indices));
    }
    artboard_move(s, &p)
}
