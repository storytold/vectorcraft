//! 3D Box tool (`box3d`) and customizable shape state (`vc-shape-tools`), a built-in add-on.
//!
//! Dragging with the 3D Box tool draws a rotatable 3D box (`addon.shapes.box3d`) whose X, Y and Z
//! rotation angles and parallel/perspective projection flags are configurable in the Control bar
//! (`addon.shapes.configure` and `tool.setOption`). Extended parameters for rectangles, ellipses,
//! polygons, stars, spirals and 3D boxes are stored in the object's `vc-shape-tools` attribute
//! entry (left out of SVG export) while keeping standard `LiveShape` variants synced.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, Node, NodeKind};
use vectorcraft_geom::corners::cut_corners;
use vectorcraft_geom::shapes::{self, CornerKind, EllipseMode};
use vectorcraft_geom::{Affine, PathData, Point, Rect, Vec2};

use crate::guides::{DrawSnap, Leave};
use crate::shape::{box_params, space_moves, turned_box};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The 3D Box tool's id.
pub const ID: &str = "box3d";

/// Object data entry (`Node::extras`) that stores extended customizable shape parameters.
/// The `vc-` prefix marks it as internal so SVG export omits it from `data-*` attributes.
pub const KEY: &str = "vc-shape-tools";

/// Maximum stored JSON length read back from a file.
const MAX_STORED: usize = 4096;

fn whole_mode() -> EllipseMode {
    EllipseMode::Whole
}

fn default_pie() -> (f64, f64) {
    (0.0, 360.0)
}

/// Parse an [`EllipseMode`] from a command or tool option string (`"slice"`/`"pie"`, `"arc"`/`"open"`,
/// `"chord"`, `"whole"`/`"full"`/`"closed"`).
pub fn parse_arc_mode(s: &str) -> Option<EllipseMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "slice" | "pie" => Some(EllipseMode::Slice),
        "arc" | "open" => Some(EllipseMode::Arc),
        "chord" => Some(EllipseMode::Chord),
        "whole" | "full" | "closed" => Some(EllipseMode::Whole),
        _ => None,
    }
}

/// Canonical string name for an [`EllipseMode`].
pub fn arc_mode_str(m: EllipseMode) -> &'static str {
    match m {
        EllipseMode::Slice => "slice",
        EllipseMode::Arc => "arc",
        EllipseMode::Chord => "chord",
        EllipseMode::Whole => "whole",
    }
}

/// Customizable shape state for rectangles, ellipses, polygons, stars, spirals and 3D boxes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "camelCase")]
pub enum CustomShape {
    Rectangle {
        w: f64,
        h: f64,
        radii: [f64; 4],
        #[serde(default)]
        kinds: [CornerKind; 4],
        #[serde(default)]
        radii_relative: bool,
        xf: Affine,
        #[serde(default = "default_pie")]
        pie: (f64, f64),
        #[serde(default = "whole_mode")]
        arc_mode: EllipseMode,
    },
    Ellipse {
        w: f64,
        h: f64,
        #[serde(default = "default_pie")]
        pie: (f64, f64),
        #[serde(default)]
        arc_mode: EllipseMode,
        xf: Affine,
    },
    Polygon {
        radius: f64,
        sides: u32,
        xf: Affine,
        #[serde(default)]
        radii: Vec<f64>,
        #[serde(default)]
        kinds: Vec<CornerKind>,
        #[serde(default)]
        radii_relative: bool,
        #[serde(default)]
        spoke_ratio: Option<f64>,
        #[serde(default)]
        rounded: f64,
        #[serde(default)]
        randomized: f64,
        #[serde(default = "default_pie")]
        pie: (f64, f64),
        #[serde(default = "whole_mode")]
        arc_mode: EllipseMode,
    },
    Spiral {
        center: Point,
        radius: f64,
        turns: f64,
        divergence: f64,
        inner_radius: f64,
        clockwise: bool,
    },
    Box3d {
        w: f64,
        h: f64,
        angle_x: f64,
        angle_y: f64,
        angle_z: f64,
        parallel_x: bool,
        parallel_y: bool,
        parallel_z: bool,
        xf: Affine,
    },
}

impl CustomShape {
    /// Regenerate the path for this customizable shape.
    pub fn to_path(&self) -> PathData {
        match self {
            CustomShape::Rectangle { w, h, radii, kinds, radii_relative, xf, pie, arc_mode } => {
                let base = shapes::rectangle(Rect::new(0.0, 0.0, *w, *h));
                let resolved = vectorcraft_doc::corners::resolve_stored_radii(&base, radii, *radii_relative);
                let r = std::array::from_fn(|i| resolved.get(i).copied().unwrap_or(0.0));
                let rect_path = shapes::rectangle_with_corners(Rect::new(0.0, 0.0, *w, *h), r, *kinds);
                let cut = shapes::apply_shape_arc(rect_path, Point::new(*w / 2.0, *h / 2.0), pie.0, pie.1, *arc_mode);
                cut.transformed(*xf)
            }
            CustomShape::Ellipse { w, h, pie, arc_mode, xf } => {
                shapes::ellipse_arc(Rect::new(0.0, 0.0, *w, *h), pie.0, pie.1, *arc_mode).transformed(*xf)
            }
            CustomShape::Polygon { radius, sides, xf, radii, kinds, radii_relative, spoke_ratio, rounded, randomized, pie, arc_mode } => {
                let raw = match spoke_ratio {
                    Some(ratio) => {
                        let r2 = (*radius * ratio.clamp(0.01, 1.0)).max(0.01);
                        shapes::star_configured(Point::ZERO, *radius, r2, *sides, 0.0, *rounded, *randomized)
                    }
                    None => shapes::polygon_configured(Point::ZERO, *radius, *sides, 0.0, *rounded, *randomized),
                };
                let outline = raw.transformed(*xf);
                let cornered = if rounded.abs() < 1e-9 && !radii.is_empty() {
                    let resolved = vectorcraft_doc::corners::resolve_stored_radii(&outline, radii, *radii_relative);
                    cut_corners(&outline, &resolved, kinds).0
                } else {
                    outline
                };
                let center = *xf * Point::ZERO;
                shapes::apply_shape_arc(cornered, center, pie.0, pie.1, *arc_mode)
            }
            CustomShape::Spiral { center, radius, turns, divergence, inner_radius, clockwise } => {
                shapes::spiral_configured(*center, *radius, *turns, *divergence, *inner_radius, *clockwise)
            }
            CustomShape::Box3d { w, h, angle_x, angle_y, angle_z, parallel_x, parallel_y, parallel_z, xf } => {
                shapes::box3d(Rect::new(0.0, 0.0, *w, *h), *angle_x, *angle_y, *angle_z, *parallel_x, *parallel_y, *parallel_z).transformed(*xf)
            }
        }
    }

    /// Standard [`LiveShape`] representation when the shape is in a state that `LiveShape` natively
    /// models (so Live Corners and Pie widgets work on it directly).
    pub fn to_live_shape(&self) -> Option<LiveShape> {
        match self {
            CustomShape::Rectangle { w, h, radii, kinds, radii_relative, xf, arc_mode, .. } if *arc_mode == EllipseMode::Whole => {
                Some(LiveShape::Rectangle { w: *w, h: *h, radii: *radii, kinds: *kinds, radii_relative: *radii_relative, xf: *xf })
            }
            CustomShape::Ellipse { w, h, pie, arc_mode, xf } if matches!(arc_mode, EllipseMode::Whole | EllipseMode::Slice) => {
                let p = if *arc_mode == EllipseMode::Whole { (0.0, 360.0) } else { *pie };
                Some(LiveShape::Ellipse { w: *w, h: *h, pie: p, xf: *xf })
            }
            CustomShape::Polygon { radius, sides, xf, radii, kinds, radii_relative, spoke_ratio, rounded, randomized, arc_mode, .. }
                if spoke_ratio.is_none() && rounded.abs() < 1e-9 && randomized.abs() < 1e-9 && *arc_mode == EllipseMode::Whole =>
            {
                Some(LiveShape::Polygon {
                    radius: *radius,
                    sides: *sides,
                    xf: *xf,
                    radii: radii.clone(),
                    kinds: kinds.clone(),
                    radii_relative: *radii_relative,
                })
            }
            _ => None,
        }
    }

    /// Whether this shape uses extended add-on parameters beyond what `LiveShape` stores on its own.
    pub fn needs_extra_storage(&self) -> bool {
        self.to_live_shape().is_none()
    }

    /// Write this shape's regenerated path, `LiveShape` (if standard) and `vc-shape-tools` attribute
    /// entry onto path node `n`.
    pub fn write_to_node(&self, n: &mut Node) {
        let new_path = self.to_path();
        let new_live = self.to_live_shape().map(Box::new);
        if let NodeKind::Path { path, live, .. } = &mut n.kind {
            *path = new_path;
            *live = new_live;
        }
        if self.needs_extra_storage() {
            if let Ok(entry) = serde_json::to_string(self) {
                n.edit_attrs(|a| match a.data.iter_mut().find(|(k, _)| k == KEY) {
                    Some(e) => e.1 = entry,
                    None => a.data.push((KEY.into(), entry)),
                });
            }
        } else {
            n.edit_attrs(|a| a.data.retain(|(k, _)| k != KEY));
        }
    }

    /// Read a [`CustomShape`] from `n`: either from its `vc-shape-tools` attribute entry (synced with
    /// `LiveShape` if both exist), from its `LiveShape`, or by detecting a star path.
    pub fn read(n: &Node) -> Option<Self> {
        let NodeKind::Path { path, live, guide: false, .. } = &n.kind else { return None };
        let stored = n
            .extras()
            .attrs
            .as_deref()
            .and_then(|a| a.data.iter().find(|(k, _)| k == KEY))
            .filter(|(_, text)| text.len() <= MAX_STORED)
            .and_then(|(_, text)| serde_json::from_str::<CustomShape>(text).ok());
        match (stored, live.as_deref()) {
            (Some(s), None) => Some(s),
            (stored_opt, Some(l)) => Self::from_live_shape(l, stored_opt),
            (None, None) => Self::detect_star_from_path(path),
        }
    }

    /// Read a [`CustomShape`] from `n`, or upgrade an open spiral path when spiral parameters are
    /// being configured.
    pub fn read_for_update(n: &Node, spiral_hint: bool) -> Option<Self> {
        if let Some(cs) = Self::read(n) {
            return Some(cs);
        }
        if spiral_hint
            && let NodeKind::Path { path, live: None, guide: false, .. } = &n.kind
            && let Some(sp) = path.subpaths.first()
            && !sp.closed
            && sp.anchors.len() >= 3
            && let Some(last) = sp.anchors.last()
            && let Some(first) = sp.anchors.first()
        {
            let center = last.p;
            let radius = first.p.distance(center).max(1.0);
            return Some(CustomShape::Spiral {
                center,
                radius,
                turns: ((sp.anchors.len().saturating_sub(1)) as f64 / 4.0).max(0.25),
                divergence: 1.0,
                inner_radius: 0.0,
                clockwise: true,
            });
        }
        None
    }

    fn from_live_shape(l: &LiveShape, stored: Option<Self>) -> Option<Self> {
        match l.folded() {
            LiveShape::Rectangle { w, h, radii, kinds, radii_relative, xf } => {
                let (pie, arc_mode) = match stored {
                    Some(CustomShape::Rectangle { pie, arc_mode, .. }) => (pie, arc_mode),
                    _ => ((0.0, 360.0), EllipseMode::Whole),
                };
                Some(CustomShape::Rectangle { w, h, radii, kinds, radii_relative, xf, pie, arc_mode })
            }
            LiveShape::Ellipse { w, h, pie, xf } => {
                let span = (pie.1 - pie.0).rem_euclid(360.0);
                let default_mode = if span < 1e-9 || 360.0 - span < 1e-9 { EllipseMode::Whole } else { EllipseMode::Slice };
                let arc_mode = match stored {
                    Some(CustomShape::Ellipse { arc_mode, .. }) if default_mode != EllipseMode::Slice => arc_mode,
                    _ => default_mode,
                };
                Some(CustomShape::Ellipse { w, h, pie, arc_mode, xf })
            }
            LiveShape::Polygon { radius, sides, xf, radii, kinds, radii_relative } => {
                let (spoke_ratio, rounded, randomized, pie, arc_mode) = match stored {
                    Some(CustomShape::Polygon { spoke_ratio, rounded, randomized, pie, arc_mode, .. }) => {
                        (spoke_ratio, rounded, randomized, pie, arc_mode)
                    }
                    _ => (None, 0.0, 0.0, (0.0, 360.0), EllipseMode::Whole),
                };
                Some(CustomShape::Polygon { radius, sides, xf, radii, kinds, radii_relative, spoke_ratio, rounded, randomized, pie, arc_mode })
            }
            LiveShape::Line { .. } | LiveShape::Path { .. } => stored,
        }
    }

    /// Detect a regular star path (even vertex count ≥ 6 with alternating outer/inner radii around
    /// its centroid) so plain stars created by `shape.star` can be customized live.
    pub fn detect_star_from_path(path: &PathData) -> Option<Self> {
        let sp = path.subpaths.first()?;
        let n = sp.anchors.len();
        if path.subpaths.len() != 1 || !sp.closed || n < 6 || !n.is_multiple_of(2) {
            return None;
        }
        let mut cx = 0.0;
        let mut cy = 0.0;
        for a in &sp.anchors {
            cx += a.p.x;
            cy += a.p.y;
        }
        let center = Point::new(cx / (n as f64), cy / (n as f64));
        let r0 = sp.anchors.first()?.p.distance(center);
        let r1 = sp.anchors.get(1)?.p.distance(center);
        let (r_outer, r_inner) = if r0 >= r1 { (r0, r1) } else { (r1, r0) };
        if r_outer <= 1e-3 || (r_outer - r_inner).abs() <= 1e-3 * r_outer {
            return None;
        }
        for (i, a) in sp.anchors.iter().enumerate() {
            let d = a.p.distance(center);
            let target = if i % 2 == 0 { r0 } else { r1 };
            if (d - target).abs() > 0.15 * r_outer {
                return None;
            }
        }
        let ratio = (r_inner / r_outer).clamp(0.01, 1.0);
        let first = sp.anchors.first()?.p - center;
        let rot = first.y.atan2(first.x) + std::f64::consts::FRAC_PI_2;
        Some(CustomShape::Polygon {
            radius: r_outer,
            sides: (n / 2) as u32,
            xf: Affine::translate((center.x, center.y)) * Affine::rotate(rot),
            radii: vec![],
            kinds: vec![],
            radii_relative: false,
            spoke_ratio: Some(ratio),
            rounded: 0.0,
            randomized: 0.0,
            pie: (0.0, 360.0),
            arc_mode: EllipseMode::Whole,
        })
    }
}

/// The customizable shapes & rotatable 3D Box add-on tool (`box3d`).
pub struct Box3dTool {
    start: Option<Point>,
    last: Point,
    mods: Mods,
    began: bool,
    pub shape: &'static str,
    pub corner_radius: f64,
    pub corner_radii: [f64; 4],
    pub corner_symmetry: bool,
    pub arc_start: f64,
    pub arc_end: f64,
    pub arc_mode: &'static str,
    pub sides: u32,
    pub points: u32,
    pub poly_is_star: bool,
    pub star_ratio: f64,
    pub rounded: f64,
    pub randomized: f64,
    pub turns: f64,
    pub divergence: f64,
    pub inner_radius: f64,
    pub clockwise: bool,
    pub angle_x: f64,
    pub angle_y: f64,
    pub angle_z: f64,
    pub parallel_x: bool,
    pub parallel_y: bool,
    pub parallel_z: bool,
    snap: DrawSnap,
}

impl Default for Box3dTool {
    fn default() -> Self {
        Self {
            start: None,
            last: Point::ZERO,
            mods: Mods::default(),
            began: false,
            shape: "box3d",
            corner_radius: 12.0,
            corner_radii: [12.0; 4],
            corner_symmetry: true,
            arc_start: 0.0,
            arc_end: 0.0,
            arc_mode: "whole",
            sides: 6,
            points: 5,
            poly_is_star: false,
            star_ratio: 0.5,
            rounded: 0.0,
            randomized: 0.0,
            turns: 3.0,
            divergence: 1.0,
            inner_radius: 0.0,
            clockwise: true,
            angle_x: 30.0,
            angle_y: 35.0,
            angle_z: 0.0,
            parallel_x: true,
            parallel_y: true,
            parallel_z: true,
            snap: DrawSnap::default(),
        }
    }
}

impl Box3dTool {
    pub fn command_at(&self, start: Point, p: Point, m: Mods, base: f64) -> (String, Value) {
        let mut out = match self.shape {
            "rectangle" => {
                let mut v = box_params(start, p, m, base);
                if self.corner_symmetry {
                    v["radius"] = json!(self.corner_radius);
                } else {
                    v["radii"] = json!(self.corner_radii);
                }
                ("addon.shapes.rectangle".into(), v)
            }
            "ellipse" => {
                let v = box_params(start, p, m, base);
                ("addon.shapes.ellipse".into(), v)
            }
            "polygon" | "star" => {
                let d = p - start;
                let r = d.hypot().max(0.01);
                let rot = if m.shift { vectorcraft_geom::constrain_rotation(base).unwrap_or(0.0) } else { (d.atan2().to_degrees() + 90.0) % 360.0 };
                let is_star = self.shape == "star" || self.poly_is_star;
                let count = if is_star { self.points } else { self.sides };
                let mut v = if is_star {
                    json!({
                        "cx": start.x,
                        "cy": start.y,
                        "radius1": r,
                        "radius2": r * self.star_ratio,
                        "points": count,
                        "rotation": rot,
                    })
                } else {
                    json!({
                        "cx": start.x,
                        "cy": start.y,
                        "radius": r,
                        "sides": count,
                        "rotation": rot,
                    })
                };
                if self.rounded.abs() > 1e-6 {
                    v["rounded"] = json!(self.rounded);
                }
                if self.randomized.abs() > 1e-6 {
                    v["randomized"] = json!(self.randomized);
                }
                (if is_star { "addon.shapes.star" } else { "addon.shapes.polygon" }.into(), v)
            }
            "spiral" => {
                let r = p.distance(start).max(0.01);
                (
                    "addon.shapes.spiral".into(),
                    json!({
                        "cx": start.x,
                        "cy": start.y,
                        "radius": r,
                        "turns": self.turns,
                        "divergence": self.divergence,
                        "innerRadius": self.inner_radius,
                        "clockwise": self.clockwise,
                    }),
                )
            }
            _ => {
                let mut v = box_params(start, p, m, base);
                v["angleX"] = json!(self.angle_x);
                v["angleY"] = json!(self.angle_y);
                v["angleZ"] = json!(self.angle_z);
                v["parallelX"] = json!(self.parallel_x);
                v["parallelY"] = json!(self.parallel_y);
                v["parallelZ"] = json!(self.parallel_z);
                return ("addon.shapes.box3d".into(), v);
            }
        };
        if matches!(self.shape, "rectangle" | "ellipse" | "polygon" | "star") {
            let span = (self.arc_end - self.arc_start).rem_euclid(360.0);
            if self.arc_mode != "whole" || (span > 1e-4 && (span - 360.0).abs() > 1e-4) {
                out.1["startAngle"] = json!(self.arc_start);
                out.1["endAngle"] = json!(self.arc_end);
                out.1["arcMode"] = json!(self.arc_mode);
            }
        }
        out
    }

    fn undo_label(&self) -> &'static str {
        match self.shape {
            "rectangle" => "Rounded Rectangle",
            "ellipse" => "Ellipse",
            "polygon" => "Polygon",
            "star" => "Star",
            "spiral" => "Spiral",
            _ => "3D Box",
        }
    }
}

impl Tool for Box3dTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.start.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => {
                self.snap.hover(cx, ev.pos, &[], None);
                vec![]
            }
            PointerKind::Down => {
                let p = self.snap.press(cx, ev.pos, &[], None);
                self.start = Some(p);
                self.last = ev.pos;
                self.began = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some(mut s) = self.start else { return vec![] };
                let radial = matches!(self.shape, "polygon" | "star" | "spiral");
                let leave = (!radial).then(|| Leave::diagonal(s, ev.mods.shift && vectorcraft_geom::constrain_axes(cx.constrain_angle).is_none()));
                let pos = self.snap.drag(cx, ev.pos, leave.as_ref());
                let ev = &PointerEvent { pos, ..*ev };
                space_moves(&mut s, &mut self.last, ev, self.began);
                self.start = Some(s);
                self.mods = ev.mods;
                let mut out = vec![];
                if !self.began {
                    if ev.pos.distance(s) < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    out.push(Action::Begin(self.undo_label().into()));
                }
                let (c, v) = self.command_at(s, ev.pos, ev.mods, cx.constrain_angle);
                out.push(Action::Preview(c, v));
                out
            }
            PointerKind::Up => {
                let Some(s) = self.start.take() else { return vec![] };
                self.snap.clear();
                if self.began {
                    self.began = false;
                    vec![Action::Commit]
                } else {
                    let (c, v) = self.command_at(s, s + Vec2::new(100.0, 100.0), ev.mods, cx.constrain_angle);
                    vec![Action::Begin(self.undo_label().into()), Action::Preview(c, v), Action::Commit]
                }
            }
            _ => vec![],
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        let Some(s) = self.start.filter(|_| self.began) else { return vec![] };
        let changed = match (self.shape, key) {
            ("polygon", ToolKey::Up) => {
                self.sides = (self.sides + 1).min(1000);
                true
            }
            ("polygon", ToolKey::Down) => {
                self.sides = self.sides.saturating_sub(1).max(3);
                true
            }
            ("star", ToolKey::Up) => {
                self.points = (self.points + 1).min(1000);
                true
            }
            ("star", ToolKey::Down) => {
                self.points = self.points.saturating_sub(1).max(3);
                true
            }
            (_, ToolKey::Up) => {
                self.angle_x = (self.angle_x + 5.0).clamp(-180.0, 180.0);
                true
            }
            (_, ToolKey::Down) => {
                self.angle_x = (self.angle_x - 5.0).clamp(-180.0, 180.0);
                true
            }
            (_, ToolKey::Right) => {
                self.angle_y = (self.angle_y + 5.0).clamp(-180.0, 180.0);
                true
            }
            (_, ToolKey::Left) => {
                self.angle_y = (self.angle_y - 5.0).clamp(-180.0, 180.0);
                true
            }
            _ => false,
        };
        if changed {
            let (c, v) = self.command_at(s, self.last, self.mods, cx.constrain_angle);
            vec![Action::Preview(c, v)]
        } else if key == ToolKey::Escape {
            self.start = None;
            self.began = false;
            self.snap.clear();
            vec![Action::Cancel]
        } else {
            vec![]
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        if let Some(s) = self.start.filter(|_| self.began && cx.measurement_labels) {
            let p = self.last + Vec2::new(cx.tol(12.0), cx.tol(12.0));
            let (r, _) = turned_box(s, self.last, self.mods, cx.constrain_angle);
            o.push(Overlay::Measure { p, text: cx.size_label(r.width(), r.height()) });
        }
        o
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }

    fn options(&self) -> Value {
        json!({
            "shape": self.shape,
            "cornerRadius": self.corner_radius,
            "cornerRadii": self.corner_radii,
            "cornerSymmetry": self.corner_symmetry,
            "arcStart": self.arc_start,
            "arcEnd": self.arc_end,
            "arcMode": self.arc_mode,
            "sides": self.sides,
            "points": self.points,
            "polyIsStar": self.poly_is_star,
            "starRatio": self.star_ratio,
            "rounded": self.rounded,
            "randomized": self.randomized,
            "turns": self.turns,
            "divergence": self.divergence,
            "innerRadius": self.inner_radius,
            "clockwise": self.clockwise,
            "angleX": self.angle_x,
            "angleY": self.angle_y,
            "angleZ": self.angle_z,
            "parallelX": self.parallel_x,
            "parallelY": self.parallel_y,
            "parallelZ": self.parallel_z,
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        let finite = v.as_f64().filter(|n| n.is_finite());
        match key {
            "shape" => {
                if let Some(s) = v.as_str() {
                    self.shape = match s {
                        "rectangle" => "rectangle",
                        "ellipse" => "ellipse",
                        "polygon" => "polygon",
                        "star" => "star",
                        "spiral" => "spiral",
                        _ => "box3d",
                    };
                }
            }
            "cornerRadius" => {
                self.corner_radius = finite.unwrap_or(self.corner_radius).clamp(0.0, crate::shape::MAX_CORNER_RADIUS);
                if self.corner_symmetry {
                    self.corner_radii = [self.corner_radius; 4];
                }
            }
            "cornerSymmetry" => {
                self.corner_symmetry = v.as_bool().unwrap_or(self.corner_symmetry);
                if self.corner_symmetry {
                    self.corner_radii = [self.corner_radius; 4];
                }
            }
            "cornerRadii" => {
                if let Some(arr) = v.as_array()
                    && arr.len() == 4
                {
                    for (i, item) in arr.iter().enumerate() {
                        if let Some(r) = item.as_f64().filter(|n| n.is_finite()) {
                            self.corner_radii[i] = r.clamp(0.0, crate::shape::MAX_CORNER_RADIUS);
                        }
                    }
                    self.corner_radius = self.corner_radii[0];
                }
            }
            "arcStart" => self.arc_start = finite.map_or(self.arc_start, |n| n.rem_euclid(360.0)),
            "arcEnd" => self.arc_end = finite.map_or(self.arc_end, |n| n.rem_euclid(360.0)),
            "arcMode" => {
                if let Some(s) = v.as_str() {
                    self.arc_mode = match s {
                        "pie" => "pie",
                        "chord" => "chord",
                        "arc" => "arc",
                        _ => "whole",
                    };
                }
            }
            "sides" => self.sides = v.as_u64().unwrap_or(6).clamp(3, 1000) as u32,
            "points" => self.points = v.as_u64().unwrap_or(5).clamp(3, 1000) as u32,
            "polyIsStar" => self.poly_is_star = v.as_bool().unwrap_or(self.poly_is_star),
            "starRatio" => self.star_ratio = finite.unwrap_or(self.star_ratio).clamp(0.01, 1.0),
            "rounded" => self.rounded = finite.unwrap_or(self.rounded).clamp(-10.0, 10.0),
            "randomized" => self.randomized = finite.unwrap_or(self.randomized).clamp(-10.0, 10.0),
            "turns" => self.turns = finite.unwrap_or(self.turns).clamp(0.01, 1024.0),
            "divergence" => self.divergence = finite.unwrap_or(self.divergence).clamp(0.0, 1000.0),
            "innerRadius" => self.inner_radius = finite.unwrap_or(self.inner_radius).clamp(0.0, 0.999),
            "clockwise" => self.clockwise = v.as_bool().unwrap_or(self.clockwise),
            "angleX" => self.angle_x = finite.unwrap_or(self.angle_x).clamp(-180.0, 180.0),
            "angleY" => self.angle_y = finite.unwrap_or(self.angle_y).clamp(-180.0, 180.0),
            "angleZ" => self.angle_z = finite.unwrap_or(self.angle_z).clamp(-180.0, 180.0),
            "parallelX" => self.parallel_x = v.as_bool().unwrap_or(self.parallel_x),
            "parallelY" => self.parallel_y = v.as_bool().unwrap_or(self.parallel_y),
            "parallelZ" => self.parallel_z = v.as_bool().unwrap_or(self.parallel_z),
            _ => {}
        }
    }
}
