//! Perspective Grid: the grid model, plane homographies, overlays, and the Perspective Grid
//! (Shift+P) and Perspective Selection (Shift+V) tools.
//!
//! The grid is stored in the document (`Document.unknown["perspectiveGrid"]`) as a
//! [`PerspectiveGrid`]. Every plane is a homography `H` from plane coordinates `(u, v)` in points
//! to the page, built from homogeneous columns: `H = [c1 c2 O]` where `O` is the ground-level
//! origin (the corner where the planes meet) and `c1`/`c2` are the images of the plane axes'
//! points at infinity — a vanishing point scaled by `1/distance` for receding axes, a direction
//! for axes parallel to the picture plane. Along a receding axis `u ↦ O + (VP − O)·u/(u + d)`,
//! the projective foreshortening of a camera `d` points from the origin.
//!
//! - 1-point: Left = the flat front plane, Right = the side wall receding to `vpLeft`, Ground
//!   recedes to `vpLeft` too.
//! - 2-point: Left recedes to `vpLeft`, Right to `vpRight`, verticals stay vertical; Ground spans
//!   both vanishing points.
//! - 3-point: as 2-point, but verticals converge to `vpVertical`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, PerspectiveAttachment};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::bbox::{Handle as BoxHandle, scale_for_drag};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub mod define;
pub mod view;
pub mod widget;
pub use define::{GridDefinition, Rgb, Station};

/// Key under `Document.unknown`.
pub const DOC_KEY: &str = "perspectiveGrid";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plane {
    #[default]
    Left,
    Right,
    Ground,
    /// No active plane (widget key 4).
    None,
}

impl Plane {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "left" => Self::Left,
            "right" => Self::Right,
            "ground" | "horizontal" => Self::Ground,
            "none" => Self::None,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Ground => "ground",
            Self::None => "none",
        }
    }
    /// Illustrator's plane colours: left blue, right orange, ground green.
    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Left => [0x33, 0x66, 0xff],
            Self::Right => [0xff, 0x8c, 0x1a],
            Self::Ground => [0x2e, 0xb8, 0x4a],
            Self::None => [0x99, 0x99, 0x99],
        }
    }
    pub const ALL: [Plane; 3] = [Plane::Left, Plane::Right, Plane::Ground];
}

/// Plane Switching widget: (plane, face quad) list and the "no plane" circle (centre, radius).
pub type WidgetGeom = (Vec<(Plane, [Point; 4])>, (Point, f64));

pub use vectorcraft_geom::Homography;

mod tools;

#[cfg(test)]
use tools::resize_cursor;
pub use tools::{PerspectiveGridTool, PerspectiveSelectionTool};

fn two() -> u8 {
    2
}
fn yes() -> bool {
    true
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// The perspective grid definition (Define Grid) plus view state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerspectiveGrid {
    /// 1, 2 or 3-point perspective.
    #[serde(default = "two")]
    pub kind: u8,
    /// Ground-level origin (where the planes meet), page coordinates.
    pub origin: [f64; 2],
    /// Horizon height (page y).
    pub horizon: f64,
    /// Left vanishing point x (the single vanishing point in 1-point perspective).
    pub vp_left: f64,
    /// Right vanishing point x.
    pub vp_right: f64,
    /// Third (vertical) vanishing point, 3-point only.
    pub vp_vertical: [f64; 2],
    /// Viewing distance (points): how fast receding axes foreshorten.
    pub distance: f64,
    /// Gridline every (points, in plane units).
    pub cell: f64,
    /// Horizontal extent of the planes (points, plane units).
    pub extent: f64,
    /// Vertical extent of the wall planes (points, plane units).
    pub height: f64,
    #[serde(default = "yes")]
    pub visible: bool,
    /// Active plane (Plane Switching widget).
    #[serde(default)]
    pub plane: Plane,
    /// Objects attached to planes (node id → plane).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attached: BTreeMap<String, Plane>,
    /// The preset the grid came from ("" = custom).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Viewing angle in degrees (two/three-point): where the station point stands between the
    /// vanishing points ([`Station`]). None for grids made before it: they foreshorten every axis
    /// by `distance`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
    /// The unit Define Grid measures in (a unit name, e.g. `points`, `inches`).
    #[serde(default = "define::points", skip_serializing_if = "define::is_points")]
    pub units: String,
    /// Scale: `[artboard, real world]` lengths (Define Grid's real-world lengths over this).
    #[serde(default = "define::one_to_one", skip_serializing_if = "define::is_one_to_one")]
    pub scale: [f64; 2],
    /// Gridline colours of the left, right and horizontal (ground) planes.
    #[serde(default = "define::left_rgb")]
    pub left_color: Rgb,
    #[serde(default = "define::right_rgb")]
    pub right_color: Rgb,
    #[serde(default = "define::ground_rgb")]
    pub ground_color: Rgb,
    /// Gridline opacity, 0–100 %.
    #[serde(default = "define::half")]
    pub opacity: f64,
    /// Where the left, right and horizontal planes were moved along their normals (points; 0: their
    /// place in the grid's definition): the plane widgets and `perspective.plane.move` set them.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub left_offset: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub right_offset: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ground_offset: f64,
    /// View → Perspective Grid → Lock Grid: the grid's widgets can't be dragged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// Lock Station Point: dragging one vanishing point moves the other around the station point.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lock_station: bool,
    /// Snap to Grid: art drawn or moved in perspective lands on gridlines within a quarter cell.
    #[serde(default = "yes")]
    pub snap: bool,
    /// Show Rulers: a ruler up the line where the planes meet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rulers: bool,
    /// Horizontal extent of the right plane (points, plane units); None: `extent`, as the left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extent_right: Option<f64>,
}

impl PerspectiveGrid {
    /// Illustrator-like preset for `kind` fitted to an artboard.
    pub fn preset(kind: u8, ab: Rect) -> Self {
        let (w, h) = (ab.width().max(1.0), ab.height().max(1.0));
        let kind = kind.clamp(1, 3);
        let horizon = ab.y0 + 0.42 * h;
        let ground = ab.y0 + 0.78 * h;
        let ox = if kind == 1 { ab.x0 + 0.3 * w } else { ab.x0 + 0.5 * w };
        let (vl, vr) = if kind == 1 { (ab.x0 + 0.55 * w, ab.x1) } else { (ab.x0 + 0.02 * w, ab.x1 - 0.02 * w) };
        Self {
            kind,
            origin: [ox, ground],
            horizon,
            vp_left: vl,
            vp_right: vr,
            vp_vertical: [ox, horizon - 3.0 * (ground - horizon)],
            distance: 0.5 * w,
            cell: (w / 30.0).max(1.0).round(),
            extent: 0.6 * w,
            height: (ground - ab.y0) * 0.8,
            visible: true,
            plane: Plane::Left,
            attached: BTreeMap::new(),
            name: String::new(),
            angle: None,
            units: define::points(),
            scale: define::one_to_one(),
            left_color: define::left_rgb(),
            right_color: define::right_rgb(),
            ground_color: define::ground_rgb(),
            opacity: define::half(),
            left_offset: 0.0,
            right_offset: 0.0,
            ground_offset: 0.0,
            locked: false,
            lock_station: false,
            snap: true,
            rulers: false,
            extent_right: None,
        }
    }

    /// Where `plane` is along its normal (points; 0: its place in the grid's definition).
    pub fn offset(&self, plane: Plane) -> f64 {
        match plane {
            Plane::Left => self.left_offset,
            Plane::Right => self.right_offset,
            Plane::Ground => self.ground_offset,
            Plane::None => 0.0,
        }
    }

    /// Move `plane` to `offset` along its normal.
    pub fn set_offset(&mut self, plane: Plane, offset: f64) {
        match plane {
            Plane::Left => self.left_offset = offset,
            Plane::Right => self.right_offset = offset,
            Plane::Ground => self.ground_offset = offset,
            Plane::None => {}
        }
    }

    /// The document's grid, if one was defined, with [`Self::attached`] collected from the objects.
    pub fn from_doc(doc: &Document) -> Option<Self> {
        let mut g = Self::stored(doc)?;
        g.collect_attached(doc);
        Some(g)
    }

    /// The document's grid, or the default two-point preset for the first artboard (hidden), with
    /// [`Self::attached`] collected from the objects.
    pub fn effective(doc: &Document) -> Self {
        let mut g = Self::current(doc);
        g.collect_attached(doc);
        g
    }

    /// [`Self::effective`] without walking the document for attached objects (for drawing and
    /// tools: ask single objects with [`Self::attachment_of`]).
    pub fn current(doc: &Document) -> Self {
        Self::stored(doc).unwrap_or_else(|| Self { visible: false, ..Self::normal(2, define::first_artboard(doc)) })
    }

    /// The grid as stored. Its `attached` holds what files from before attachments were kept on the
    /// objects recorded.
    fn stored(doc: &Document) -> Option<Self> {
        doc.unknown.get(DOC_KEY).and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    fn collect_attached(&mut self, doc: &Document) {
        doc.walk(|n| {
            if let Some((pl, _)) = attachment(n) {
                self.attached.insert(n.id.0.to_string(), pl);
            }
        });
    }

    /// Store into the document. Attachments live on the objects: the stored grid keeps none.
    pub fn store(&self, doc: &mut Document) {
        self.adopt_stored_attachments(doc);
        let mut g = self.clone();
        g.attached.clear();
        if let Ok(v) = serde_json::to_value(&g) {
            doc.unknown.insert(DOC_KEY.into(), v);
        }
    }

    /// Move the attachments a grid stored before they were kept on the objects onto the objects,
    /// unless this grid no longer has them (released since); ids of deleted objects are dropped.
    pub fn adopt_stored_attachments(&self, doc: &mut Document) {
        let Some(old) = Self::stored(doc) else { return };
        for (k, pl) in &old.attached {
            if self.attached.get(k) != Some(pl) {
                continue;
            }
            if let Some(n) = k.parse::<u64>().ok().and_then(|id| doc.node_mut(NodeId(id)))
                && n.extras().perspective.is_none()
            {
                n.extras_mut().perspective = Some(Box::new(PerspectiveAttachment::new(pl.id(), 0.0)));
            }
        }
        if let Some(v) = doc.unknown.get_mut(DOC_KEY).and_then(Value::as_object_mut) {
            v.remove("attached");
        }
    }

    /// Definition only (for tool previews and the Define Grid dialog).
    pub fn definition_json(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            o.remove("attached");
        }
        v
    }

    /// Merge `patch` (any subset of the fields) into this grid.
    pub fn merged(&self, patch: &Value) -> Result<Self, String> {
        let mut v = serde_json::to_value(self).map_err(|e| e.to_string())?;
        if let (Some(o), Some(p)) = (v.as_object_mut(), patch.as_object()) {
            for (k, val) in p {
                if k == "ids" || k == "attached" {
                    continue;
                }
                o.insert(k.clone(), val.clone());
            }
        }
        let mut g: Self = serde_json::from_value(v).map_err(|e| e.to_string())?;
        g.reconcile(self, patch);
        g.validate()?;
        Ok(g)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3).contains(&self.kind) {
            return Err("kind must be 1, 2 or 3".into());
        }
        let nums = [
            self.origin[0],
            self.origin[1],
            self.horizon,
            self.vp_left,
            self.vp_right,
            self.vp_vertical[0],
            self.vp_vertical[1],
            self.distance,
            self.cell,
            self.extent,
            self.height,
        ];
        if nums.iter().any(|v| !v.is_finite() || v.abs() > 4.0e6) {
            return Err("grid values must be finite".into());
        }
        if self.distance <= 0.0 || self.cell <= 0.0 || self.extent <= 0.0 || self.height <= 0.0 {
            return Err("distance, cell, extent and height must be positive".into());
        }
        if (self.origin[1] - self.horizon).abs() < 1e-6 {
            return Err("ground level must differ from the horizon".into());
        }
        // A moved plane stays in front of the viewer.
        let behind = |p: Plane| self.axes(p).is_some_and(|[.., n]| 1.0 + self.offset(p) * self.axis_col(n)[2] <= 1e-9);
        if Plane::ALL.iter().any(|p| !self.offset(*p).is_finite() || self.offset(*p).abs() > MAX_DEPTH || behind(*p)) {
            return Err("a plane is moved too far (behind the viewer)".into());
        }
        self.validate_definition()
    }

    pub fn planes(&self) -> [Plane; 3] {
        Plane::ALL
    }

    /// The image of `axis`'s point at infinity: a vanishing point scaled by `1/distance` for a
    /// receding axis, a direction for an axis parallel to the picture plane.
    fn axis_col(&self, axis: Axis) -> [f64; 3] {
        // Each receding axis foreshortens by the viewer's distance to its vanishing point.
        let (ll, lr, lu) = self.foreshortening();
        match axis {
            Axis::X if self.kind == 1 => [1.0, 0.0, 0.0],
            Axis::X => [self.vp_right * lr, self.horizon * lr, lr],
            Axis::Y if self.kind == 3 => [self.vp_vertical[0] * lu, self.vp_vertical[1] * lu, lu],
            Axis::Y => [0.0, -1.0, 0.0],
            Axis::Z => [self.vp_left * ll, self.horizon * ll, ll],
        }
    }

    /// A plane's axes: `[u, v, normal]`.
    pub fn axes(&self, plane: Plane) -> Option<[Axis; 3]> {
        Some(match (self.kind, plane) {
            (_, Plane::None) => return None,
            (1, Plane::Left) => [Axis::X, Axis::Y, Axis::Z],
            (1, Plane::Right) => [Axis::Z, Axis::Y, Axis::X],
            (_, Plane::Left) => [Axis::Z, Axis::Y, Axis::X],
            (_, Plane::Right) => [Axis::X, Axis::Y, Axis::Z],
            (_, Plane::Ground) => [Axis::X, Axis::Z, Axis::Y],
        })
    }

    /// The homography of `plane` where it is now (plane coordinates in points → page).
    pub fn homography(&self, plane: Plane) -> Option<Homography> {
        self.homography_at(plane, self.offset(plane))
    }

    /// The homography of the plane parallel to `plane` that lies `depth` points along its normal
    /// (`None` behind the viewer).
    pub fn homography_at(&self, plane: Plane, depth: f64) -> Option<Homography> {
        let [u, v, n] = self.axes(plane)?;
        self.frame_homography(u, v, (n, depth))
    }

    /// The plane spanned by axes `u` and `v` through the point `at.1` along axis `at.0`.
    fn frame_homography(&self, u: Axis, v: Axis, at: (Axis, f64)) -> Option<Homography> {
        if !at.1.is_finite() {
            return None;
        }
        let c = self.axis_col(at.0);
        let o = [self.origin[0] + at.1 * c[0], self.origin[1] + at.1 * c[1], 1.0 + at.1 * c[2]];
        if o[2] <= 1e-9 {
            return None;
        }
        let h = Homography::from_cols(self.axis_col(u), self.axis_col(v), o);
        (h.det().abs() > 1e-15).then_some(h)
    }

    /// The plane-space offset between page points `from` and `to` on `plane` at `depth`.
    pub fn plane_delta(&self, plane: Plane, depth: f64, from: Point, to: Point) -> Option<Vec2> {
        let hi = self.homography_at(plane, depth)?.inverse()?;
        Some(hi.apply(to)? - hi.apply(from)?)
    }

    /// The depth an object on `plane` at `depth` dragged from `from` to `to` perpendicular to its
    /// plane reaches: the pointer is read on the plane through the drag start that holds the
    /// normal and the plane's v axis.
    pub fn depth_at(&self, plane: Plane, depth: f64, from: Point, to: Point) -> Option<f64> {
        let [u, v, n] = self.axes(plane)?;
        let q = self.homography_at(plane, depth)?.inverse()?.apply(from)?;
        let ci = self.frame_homography(n, v, (u, q.x))?.inverse()?;
        let d = depth + (ci.apply(to)?.x - ci.apply(from)?.x);
        (d.is_finite() && d.abs() <= MAX_DEPTH).then_some(d)
    }

    /// The page map of plane-space affine map `m` for art on `plane` at `depth`, then moved `dz`
    /// along the plane's normal: `H(depth + dz) · m · H(depth)⁻¹`.
    pub fn transform_map(&self, plane: Plane, depth: f64, m: Affine, dz: f64) -> Option<Homography> {
        let h = self.homography_at(plane, depth)?;
        let to = self.homography_at(plane, depth + dz)?;
        Some(to.then_after(&Homography::from_affine(m)).then_after(&h.inverse()?))
    }

    /// Where object `id` is attached (plane, depth): its own record, else the one a grid stored
    /// before attachments were kept on the objects.
    pub fn attachment_of(&self, doc: &Document, id: NodeId) -> Option<(Plane, f64)> {
        doc.node(id).and_then(attachment).or_else(|| self.attached_plane(id).map(|p| (p, 0.0)))
    }

    /// The plane-space bounds of the attached objects among `ids` lying on the plane of the first
    /// one: (that plane, the first one's depth, bounds).
    pub fn plane_bounds(&self, doc: &Document, ids: &[NodeId]) -> Option<(Plane, f64, Rect)> {
        let mut out: Option<(Plane, f64, Rect)> = None;
        for id in ids {
            let Some((pl, depth)) = self.attachment_of(doc, *id) else { continue };
            if out.is_some_and(|o| o.0 != pl) {
                continue;
            }
            let hi = self.homography_at(pl, depth).and_then(|h| h.inverse());
            let Some(b) = hi.zip(doc.node(*id)).and_then(|(hi, n)| mapped_bounds(n, &hi)) else { continue };
            out = Some(match out {
                Some((p, d, r)) => (p, d, r.union(b)),
                None => (pl, depth, b),
            });
        }
        out
    }

    /// Plane-coordinate extent of a plane: the left plane is `extent` wide, the right one
    /// [`Self::right_extent`]; the ground spans both (one-point: the front plane's width by the
    /// side wall's depth).
    pub fn domain(&self, plane: Plane) -> Rect {
        match (plane, self.kind) {
            (Plane::Ground, 1) => Rect::new(0.0, 0.0, self.extent, self.right_extent()),
            (Plane::Ground, _) => Rect::new(0.0, 0.0, self.right_extent(), self.extent),
            (Plane::Right, _) => Rect::new(0.0, 0.0, self.right_extent(), self.height),
            _ => Rect::new(0.0, 0.0, self.extent, self.height),
        }
    }

    /// The right plane's horizontal extent.
    pub fn right_extent(&self) -> f64 {
        self.extent_right.unwrap_or(self.extent)
    }

    /// Page → plane coordinates.
    pub fn to_plane(&self, plane: Plane, p: Point) -> Option<Point> {
        self.homography(plane)?.inverse()?.apply(p)
    }
    /// Plane → page coordinates.
    pub fn to_page(&self, plane: Plane, q: Point) -> Option<Point> {
        self.homography(plane)?.apply(q)
    }

    /// The map that puts flat art with bounds `b` onto `plane`: an axis-aligned affine map sends
    /// `b` to the plane rectangle whose opposite corners project to `b`'s top-left and
    /// bottom-right, then the plane homography projects it (so a drawn rectangle keeps the two
    /// corners the user dragged between).
    pub fn attach_map(&self, plane: Plane, b: Rect) -> Option<impl Fn(Point) -> Option<Point> + use<>> {
        let h = self.attach_homography(plane, b, false)?;
        Some(move |p: Point| h.apply(p))
    }

    /// [`Self::attach_map`]; with `snap` the corners land on the nearest gridlines (Snap to Grid).
    pub fn attach_map_with(&self, plane: Plane, b: Rect, snap: bool) -> Option<impl Fn(Point) -> Option<Point> + use<>> {
        let h = self.attach_homography(plane, b, snap)?;
        Some(move |p: Point| h.apply(p))
    }

    /// [`Self::attach_map_with`] as one projective map.
    pub fn attach_homography(&self, plane: Plane, b: Rect, snap: bool) -> Option<Homography> {
        let h = self.homography(plane)?;
        let hi = h.inverse()?;
        let snapped = |q: Point| if snap { self.snap_plane(q) } else { q };
        let a = snapped(hi.apply(Point::new(b.x0, b.y0))?);
        let c = snapped(hi.apply(Point::new(b.x1, b.y1))?);
        let sx = if b.width().abs() > 1e-9 { (c.x - a.x) / b.width() } else { 1.0 };
        let sy = if b.height().abs() > 1e-9 { (c.y - a.y) / b.height() } else { sx.abs() * if c.y < a.y { -1.0 } else { 1.0 } };
        let sx = if b.width().abs() > 1e-9 { sx } else { sy.abs() };
        let m = Affine::new([sx, 0.0, 0.0, sy, a.x - b.x0 * sx, a.y - b.y0 * sy]);
        Some(h.then_after(&Homography::from_affine(m)))
    }

    /// The map that puts flat art drawn at `at` onto `plane` keeping its sizes in plane units (a
    /// click-to-size shape dialog's shape): `at` stays where it is, and right and down on the page
    /// go the ways the plane's axes run on the page there.
    /// With `snap` the click lands on the nearest gridline intersection (Snap to Grid).
    pub fn size_homography(&self, plane: Plane, at: Point, snap: bool) -> Option<Homography> {
        let h = self.homography(plane)?;
        let q = h.inverse()?.apply(at)?;
        let q = if snap { self.snap_plane(q) } else { q };
        let at = h.apply(q)?;
        let sx = if h.apply(q + Vec2::new(1.0, 0.0))?.x < at.x { -1.0 } else { 1.0 };
        let sy = if h.apply(q + Vec2::new(0.0, 1.0))?.y < at.y { -1.0 } else { 1.0 };
        let m = Affine::new([sx, 0.0, 0.0, sy, q.x - sx * at.x, q.y - sy * at.y]);
        Some(h.then_after(&Homography::from_affine(m)))
    }

    /// The map that slides art lying on `plane` by the plane-space offset between `from` and `to`.
    pub fn move_map(&self, plane: Plane, from: Point, to: Point) -> Option<impl Fn(Point) -> Option<Point> + use<>> {
        let hi = self.homography(plane)?.inverse()?;
        self.move_map_by(plane, hi.apply(to)? - hi.apply(from)?)
    }

    /// The plane object `id` is attached to, as collected by [`Self::from_doc`].
    pub fn attached_plane(&self, id: NodeId) -> Option<Plane> {
        self.attached.get(&id.0.to_string()).copied()
    }

    /// Grid lines of `plane` (page space).
    pub fn lines(&self, plane: Plane) -> Vec<(Point, Point)> {
        let Some(h) = self.homography(plane) else { return vec![] };
        let d = self.domain(plane);
        let mut out = vec![];
        // A stored grid is not checked: no spacing, or an extent that isn't a number, has no lines.
        let (w, h_len) = (d.width(), d.height());
        if !(self.cell.is_finite() && self.cell > 0.0 && w.is_finite() && h_len.is_finite()) {
            return out;
        }
        let step = |len: f64| {
            let mut s = self.cell;
            while len / s > 120.0 {
                s *= 2.0;
            }
            s
        };
        let (su, sv) = (step(w), step(h_len));
        // Counted, not summed: a count in whole lines (at most 121 a way). The fixed 1e-9
        // tolerance can inflate the count when the whole domain is far smaller than it, so the
        // bound is capped on its own, before any loop runs.
        let count = |len: f64, s: f64| ((len + 1e-9) / s).floor().clamp(0.0, 120.0) as usize;
        for i in 0..=count(w, su) {
            let u = i as f64 * su;
            if let (Some(a), Some(b)) = (h.apply(Point::new(u, 0.0)), h.apply(Point::new(u, h_len))) {
                out.push((a, b));
            }
        }
        for i in 0..=count(h_len, sv) {
            let v = i as f64 * sv;
            if let (Some(a), Some(b)) = (h.apply(Point::new(0.0, v)), h.apply(Point::new(w, v))) {
                out.push((a, b));
            }
        }
        out
    }

    /// Plane Switching widget geometry in the first artboard's top-left corner (headless): (plane,
    /// quad) faces and the "no plane" circle (centre, r). See [`Self::widget_at`].
    pub fn widget(&self, doc: &Document, tol: f64) -> WidgetGeom {
        self.widget_at(doc, tol, widget::WidgetPlace::default())
    }

    /// Which widget control is at `p`.
    pub fn widget_hit(&self, doc: &Document, tol: f64, p: Point) -> Option<Plane> {
        let (faces, (c, r)) = self.widget(doc, tol);
        if p.distance(c) <= r * 1.5 {
            return Some(Plane::None);
        }
        faces.into_iter().find(|(_, q)| in_quad(q, p)).map(|(pl, _)| pl)
    }

    /// The left, right and horizontal plane widgets: where each sits (on its plane: the middle of a
    /// wall's ground edge, the middle of the horizontal plane).
    pub fn plane_widgets(&self) -> Vec<(Plane, Point)> {
        let e = self.extent / 2.0;
        Plane::ALL
            .into_iter()
            .filter_map(|pl| {
                let q = if pl == Plane::Ground { Point::new(e, e) } else { Point::new(e, 0.0) };
                Some((pl, self.homography(pl)?.apply(q)?))
            })
            .collect()
    }
}

/// Dragging a plane widget: the plane moves along its normal by as much as the pointer moved along
/// it (from the widget, wherever on it the press was).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaneDrag {
    pub plane: Plane,
    anchor: Point,
    press: Point,
    offset: f64,
}

impl PlaneDrag {
    /// The plane widget within `tol` of `p` (drawn grids only).
    pub fn hit(g: &PerspectiveGrid, tol: f64, p: Point) -> Option<Self> {
        if !g.visible {
            return None;
        }
        let (plane, anchor) = g.plane_widgets().into_iter().find(|(_, q)| q.distance(p) <= tol)?;
        Some(Self { plane, anchor, press: p, offset: g.offset(plane) })
    }

    /// The preview for the pointer at `p`: Shift moves the objects on the plane with it, Alt copies
    /// them.
    pub fn preview(&self, g: &PerspectiveGrid, p: Point, m: Mods) -> Option<Action> {
        let offset = g.depth_at(self.plane, self.offset, self.anchor, self.anchor + (p - self.press))?;
        let objects = if m.alt {
            "copy"
        } else if m.shift {
            "move"
        } else {
            "none"
        };
        Some(Action::Preview("perspective.plane.move".into(), json!({"plane": self.plane.id(), "offset": offset, "objects": objects})))
    }

    /// The plane options dialog a double-click on the widget opens.
    pub fn options(&self) -> Action {
        Action::Dialog(PLANE_DIALOG.into(), json!({"plane": self.plane.id()}))
    }
}

/// The dialog kind of a plane's options (double-click its widget).
pub const PLANE_DIALOG: &str = "perspectivePlane";

/// The plane widgets as drawn: a diamond in each plane's colour.
fn plane_widget_overlays(g: &PerspectiveGrid, tol: f64) -> Vec<Overlay> {
    let r = 5.0 * tol;
    g.plane_widgets()
        .into_iter()
        .flat_map(|(pl, p)| {
            let [cr, cg, cb] = pl.color();
            let quad = [Point::new(p.x, p.y - r), Point::new(p.x + r, p.y), Point::new(p.x, p.y + r), Point::new(p.x - r, p.y)];
            [
                Overlay::Highlight { quad, color: [cr, cg, cb, 220] },
                Overlay::Path { path: super::diamond(p, r), color: [0x20, 0x20, 0x20], width: 1.0, dashed: false },
            ]
        })
        .collect()
}

/// How far (points) along a plane's normal objects and planes may go.
pub const MAX_DEPTH: f64 = 1.0e6;

/// An axis of the grid's 3-D frame: Y is up, X and Z recede to the right and left vanishing points
/// (in 1-point perspective X is the flat horizontal and Z recedes to the vanishing point).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// The plane and depth object `n` is attached to (its own record).
pub fn attachment(n: &Node) -> Option<(Plane, f64)> {
    let a = n.extras().perspective.as_deref()?;
    Plane::parse(&a.plane).filter(|p| *p != Plane::None).map(|p| (p, a.depth))
}

/// The objects in perspective that aren't inside another one: (id, plane, depth).
pub fn attached_roots(doc: &Document) -> Vec<(NodeId, Plane, f64)> {
    fn visit(n: &Node, out: &mut Vec<(NodeId, Plane, f64)>) {
        match attachment(n) {
            Some((pl, depth)) => out.push((n.id, pl, depth)),
            None => n.children().into_iter().flatten().for_each(|c| visit(c, out)),
        }
    }
    let mut out = vec![];
    for l in &doc.layers {
        visit(l, &mut out);
    }
    out
}

/// Attach `n` to `plane` at `depth` (type and symbols keep their projection).
pub fn set_attachment(n: &mut Node, plane: Plane, depth: f64) {
    let mut extras = n.extras_mut();
    let rec = extras.perspective.get_or_insert_with(Default::default);
    rec.plane = plane.id().into();
    rec.depth = if depth.is_finite() { depth } else { 0.0 };
}

/// Release `n` from the grid (Release with Perspective): it keeps its look, type and symbols their
/// projection.
pub fn release(n: &mut Node) {
    let mut extras = n.extras_mut();
    match extras.perspective.as_deref_mut() {
        Some(rec) if rec.projection.is_some() => *rec = PerspectiveAttachment { projection: rec.projection, ..Default::default() },
        _ => extras.perspective = None,
    }
}

/// The bounds of `n` mapped by `hi`: paths by their mapped curves, other objects by their boxes'
/// corners. `None` when part of it maps beyond the horizon.
fn mapped_bounds(n: &Node, hi: &Homography) -> Option<Rect> {
    let mut acc: Option<Rect> = None;
    let mut ok = true;
    n.walk(&mut |c| match &c.kind {
        NodeKind::Path { path, .. } => {
            let mut path = path.clone();
            for a in path.subpaths.iter_mut().flat_map(|sp| sp.anchors.iter_mut()) {
                for q in [&mut a.p, &mut a.h_in, &mut a.h_out] {
                    match hi.apply(*q) {
                        Some(v) => *q = v,
                        None => ok = false,
                    }
                }
            }
            acc = vectorcraft_geom::union_opt(acc, path.bounds());
        }
        NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Compound { .. } | NodeKind::Blend { .. } => {}
        _ => match c.geometric_bounds().map(|b| hi.map_rect_bbox(b)) {
            Some(Some(b)) => acc = vectorcraft_geom::union_opt(acc, Some(b)),
            Some(None) => ok = false,
            None => {}
        },
    });
    if ok { acc } else { None }
}

/// Is `p` inside the convex quad `q` (either winding; a point on an edge is inside)?
pub(crate) fn in_quad(q: &[Point; 4], p: Point) -> bool {
    let mut sign = 0.0;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let c = (b - a).cross(p - a);
        if c.abs() < 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

/// Grid overlays (lines, horizon, vanishing points, widget) drawn whenever the grid is visible
/// (selecting a perspective tool shows it). `tol` = document units per screen pixel.
pub fn grid_overlays(doc: &Document, tol: f64) -> Vec<Overlay> {
    grid_overlays_in(doc, tol, Some(widget::WidgetPlace::default()))
}

/// [`grid_overlays`] with the Plane Switching Widget at `place` (None: hidden).
pub fn grid_overlays_in(doc: &Document, tol: f64, place: Option<widget::WidgetPlace>) -> Vec<Overlay> {
    let g = PerspectiveGrid::current(doc);
    if !g.visible {
        return vec![];
    }
    let mut out = vec![];
    for pl in g.planes() {
        let (line, color) = (g.line_color(pl), g.plane_color(pl));
        for (a, b) in g.lines(pl) {
            out.push(Overlay::GridLine { a, b, color: line });
        }
        if pl == g.plane
            && let Some(h) = g.homography(pl)
        {
            let d = g.domain(pl);
            let corners: Vec<Point> =
                [(d.x0, d.y0), (d.x1, d.y0), (d.x1, d.y1), (d.x0, d.y1)].iter().filter_map(|&(x, y)| h.apply(Point::new(x, y))).collect();
            if corners.len() == 4 {
                out.push(Overlay::Path { path: crate::xform::polygon(&corners, true), color, width: 2.0, dashed: false });
            }
        }
    }
    let xs = [g.vp_left, g.vp_right, g.origin[0]];
    let (x0, x1) = (xs.iter().cloned().fold(f64::MAX, f64::min) - 50.0 * tol, xs.iter().cloned().fold(f64::MIN, f64::max) + 50.0 * tol);
    out.push(Overlay::Line { a: Point::new(x0, g.horizon), b: Point::new(x1, g.horizon), color: [0x60, 0x60, 0x60], dashed: true });
    if g.rulers {
        out.extend(g.ruler_overlays(tol));
    }
    // Widget.
    let Some(place) = place else { return out };
    let (faces, (c, r)) = g.widget_at(doc, tol, place);
    for (pl, q) in faces {
        let [cr, cg, cb] = pl.color();
        out.push(Overlay::Highlight { quad: q, color: [cr, cg, cb, if pl == g.plane { 230 } else { 70 }] });
        out.push(Overlay::Path { path: crate::xform::polygon(&q, true), color: [0x40, 0x40, 0x40], width: 1.0, dashed: false });
    }
    out.push(Overlay::Path {
        path: super::ellipse_path(c, r, r, 0.0),
        color: if g.plane == Plane::None { [0x20, 0x20, 0x20] } else { [0x99, 0x99, 0x99] },
        width: 1.5,
        dashed: false,
    });
    out.extend(plane_widget_overlays(&g, tol));
    out
}

// ---------- Perspective Grid tool ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    VpLeft,
    VpRight,
    VpVertical,
    Horizon,
    Origin,
    ExtentLeft,
    ExtentRight,
    Height,
    /// The left and right ground-level points: they move the whole grid.
    GroundLeft,
    GroundRight,
    /// The grid cell size widget, `n` cells up the line where the planes meet.
    Cell,
}

/// The ground-level points and the cell widget sit at least this far (screen pixels) from the
/// origin.
const NEAR_ORIGIN_PX: f64 = 24.0;

/// How many cells up the cell widget sits so it stays clear of the origin (`tol` = document units
/// per screen pixel).
fn cell_steps(g: &PerspectiveGrid, tol: f64) -> f64 {
    (NEAR_ORIGIN_PX * tol / g.cell.max(1e-9)).ceil().clamp(1.0, 1.0e6)
}

fn handles(g: &PerspectiveGrid, tol: f64) -> Vec<(Handle, Point)> {
    let o = Point::new(g.origin[0], g.origin[1]);
    let mut v = vec![(Handle::VpLeft, Point::new(g.vp_left, g.horizon))];
    if g.kind != 1 {
        v.push((Handle::VpRight, Point::new(g.vp_right, g.horizon)));
    }
    if g.kind == 3 {
        v.push((Handle::VpVertical, Point::new(g.vp_vertical[0], g.vp_vertical[1])));
    }
    v.push((Handle::Origin, o));
    let mut at = |h: Handle, pl: Plane, q: Point| {
        if let Some(p) = g.to_page(pl, q) {
            v.push((h, p));
        }
    };
    at(Handle::ExtentLeft, Plane::Left, Point::new(g.extent, 0.0));
    at(Handle::ExtentRight, Plane::Right, Point::new(g.right_extent(), 0.0));
    at(Handle::Height, Plane::Left, Point::new(0.0, g.height));
    at(Handle::Cell, Plane::Left, Point::new(0.0, g.cell * cell_steps(g, tol)));
    // The ground-level points: a little way from the origin along each wall's ground line.
    for (h, pl) in [(Handle::GroundLeft, Plane::Left), (Handle::GroundRight, Plane::Right)] {
        if let Some(p) = g.to_page(pl, Point::new(g.cell, 0.0)).filter(|p| p.distance(o) > 1e-9) {
            v.push((h, o + (p - o).normalize() * (NEAR_ORIGIN_PX * tol)));
        }
    }
    // The horizon handle sits on the horizon above the origin.
    v.push((Handle::Horizon, Point::new(g.origin[0], g.horizon)));
    v
}

/// A widget drag: the widget, the grid and the pointer when it was pressed, and the cell widget's
/// steps then.
struct Press {
    handle: Handle,
    grid: PerspectiveGrid,
    at: Point,
    steps: f64,
}

/// `g` moved by `d`: the whole grid, vanishing points and horizon too.
fn translated(g: &PerspectiveGrid, d: Vec2) -> PerspectiveGrid {
    let mut n = g.clone();
    n.origin = [g.origin[0] + d.x, g.origin[1] + d.y];
    n.horizon += d.y;
    n.vp_left += d.x;
    n.vp_right += d.x;
    n.vp_vertical = [g.vp_vertical[0] + d.x, g.vp_vertical[1] + d.y];
    n
}

/// The grid with a widget dragged to `p`. Shift constrains: a vanishing point keeps the horizon,
/// the ground-level points and the origin move along one axis, extents and the height go by whole
/// cells. Alt drags both extents together.
fn drag_handle(press: &Press, p: Point, mods: Mods) -> PerspectiveGrid {
    let g = &press.grid;
    let mut n = g.clone();
    let along_axis = |p: Point| {
        let d = p - press.at;
        if !mods.shift {
            d
        } else if d.x.abs() >= d.y.abs() {
            Vec2::new(d.x, 0.0)
        } else {
            Vec2::new(0.0, d.y)
        }
    };
    let cells = |v: f64| if mods.shift { (v / g.cell).round().max(1.0) * g.cell } else { v.max(g.cell) };
    let h = press.handle;
    match h {
        Handle::VpLeft | Handle::VpRight => {
            if !mods.shift {
                n.horizon = p.y;
            }
            match (h == Handle::VpLeft, g.lock_station && g.kind != 1) {
                (left, true) => n.swing(left, p.x),
                (true, false) => n.vp_left = p.x,
                (false, false) => n.vp_right = p.x,
            }
        }
        Handle::VpVertical => n.vp_vertical = [p.x, p.y],
        Handle::Horizon => n.horizon = p.y,
        Handle::Origin => {
            let d = along_axis(p);
            n.origin = [g.origin[0] + d.x, g.origin[1] + d.y];
        }
        Handle::GroundLeft | Handle::GroundRight => n = translated(g, along_axis(p)),
        Handle::ExtentLeft | Handle::ExtentRight => {
            let pl = if h == Handle::ExtentRight { Plane::Right } else { Plane::Left };
            if let Some(q) = g.to_plane(pl, p) {
                let e = cells(q.x);
                if h == Handle::ExtentLeft || mods.alt {
                    n.extent = e;
                }
                if h == Handle::ExtentRight || mods.alt {
                    n.extent_right = Some(e);
                }
            }
        }
        Handle::Height => {
            if let Some(q) = g.to_plane(Plane::Left, p) {
                n.height = cells(q.y);
            }
        }
        Handle::Cell => {
            if let Some(q) = g.to_plane(Plane::Left, p) {
                n.cell = (q.y / press.steps).max(0.5);
            }
        }
    }
    if n.validate().is_ok() { n } else { g.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn grid(kind: u8) -> PerspectiveGrid {
        PerspectiveGrid::preset(kind, Rect::new(0.0, 0.0, 800.0, 600.0))
    }

    #[test]
    fn homography_maps_plane_corners_to_the_vanishing_points() {
        for kind in [1u8, 2, 3] {
            let g = grid(kind);
            for pl in Plane::ALL {
                let h = g.homography(pl).unwrap();
                // The plane origin is the ground-level origin.
                let o = h.apply(Point::new(0.0, 0.0)).unwrap();
                assert!((o - Point::new(g.origin[0], g.origin[1])).hypot() < 1e-9, "{kind} {pl:?}");
                // Far along a receding axis, points approach the vanishing point on the horizon.
                let far = h.apply(Point::new(1e9, 0.0)).unwrap();
                let expect = match (kind, pl) {
                    (1, Plane::Left | Plane::Ground) => None,
                    (1, Plane::Right) => Some(Point::new(g.vp_left, g.horizon)),
                    (_, Plane::Left) => Some(Point::new(g.vp_left, g.horizon)),
                    _ => Some(Point::new(g.vp_right, g.horizon)),
                };
                if let Some(e) = expect {
                    assert!((far - e).hypot() < 1e-3, "{kind} {pl:?} {far:?}");
                }
            }
        }
        // 2-point walls: verticals stay vertical and unforeshortened at the origin edge.
        let g = grid(2);
        let top = g.to_page(Plane::Left, Point::new(0.0, 100.0)).unwrap();
        assert!((top - Point::new(g.origin[0], g.origin[1] - 100.0)).hypot() < 1e-9);
        // Foreshortening: u = distance lands halfway to the vanishing point.
        let mid = g.to_page(Plane::Right, Point::new(g.distance, 0.0)).unwrap();
        let want = Point::new(g.origin[0], g.origin[1]).lerp(Point::new(g.vp_right, g.horizon), 0.5);
        assert!((mid - want).hypot() < 1e-9);
    }

    #[test]
    fn homography_round_trips() {
        for kind in [1u8, 2, 3] {
            let g = grid(kind);
            for pl in Plane::ALL {
                for q in [Point::new(10.0, 20.0), Point::new(300.0, 5.0), Point::new(0.0, 0.0), Point::new(123.0, 77.0)] {
                    let p = g.to_page(pl, q).unwrap();
                    let back = g.to_plane(pl, p).unwrap();
                    assert!((back - q).hypot() < 1e-6, "{kind} {pl:?} {q:?} → {back:?}");
                }
            }
        }
        assert!(grid(2).homography(Plane::None).is_none());
    }

    #[test]
    fn attach_map_keeps_the_dragged_corners() {
        let g = grid(2);
        let b = Rect::new(450.0, 300.0, 550.0, 420.0);
        let m = g.attach_map(Plane::Right, b).unwrap();
        assert!((m(Point::new(b.x0, b.y0)).unwrap() - Point::new(b.x0, b.y0)).hypot() < 1e-6);
        assert!((m(Point::new(b.x1, b.y1)).unwrap() - Point::new(b.x1, b.y1)).hypot() < 1e-6);
        // The other corners lie on grid lines through the plane: the top-right corner is on the
        // line from the top-left corner to the right vanishing point.
        let tr = m(Point::new(b.x1, b.y0)).unwrap();
        let vp = Point::new(g.vp_right, g.horizon);
        assert!((tr - Point::new(b.x0, b.y0)).cross(vp - Point::new(b.x0, b.y0)).abs() < 1e-6);
        assert!(tr.y != b.y0);
    }

    #[test]
    fn move_map_slides_within_the_plane() {
        let g = grid(2);
        let from = g.to_page(Plane::Left, Point::new(50.0, 50.0)).unwrap();
        let to = g.to_page(Plane::Left, Point::new(80.0, 60.0)).unwrap();
        let m = g.move_map(Plane::Left, from, to).unwrap();
        let p = g.to_page(Plane::Left, Point::new(10.0, 10.0)).unwrap();
        let q = g.to_plane(Plane::Left, m(p).unwrap()).unwrap();
        assert!((q - Point::new(40.0, 20.0)).hypot() < 1e-6);
    }

    #[test]
    fn grid_serde_and_merge() {
        let g = grid(3);
        let mut d = Document::new(800.0, 600.0);
        g.store(&mut d);
        assert_eq!(PerspectiveGrid::from_doc(&d), Some(g.clone()));
        let m = g.merged(&json!({"kind": 1, "cell": 25})).unwrap();
        assert_eq!((m.kind, m.cell), (1, 25.0));
        assert!(g.merged(&json!({"distance": -1})).is_err());
    }

    #[test]
    fn grid_tool_widget_and_handles() {
        let mut d = Document::new(800.0, 600.0);
        grid(2).store(&mut d);
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let g = PerspectiveGrid::effective(&d);
        let mut t = PerspectiveGridTool::default();
        // The horizon handle drags up and down; in a view turned 90° it drags left and right on
        // screen.
        let horizon = handles(&g, c.tol(1.0)).into_iter().find(|h| h.0 == Handle::Horizon).map(|h| h.1).unwrap();
        assert_eq!(t.cursor(&c, horizon, Mods::default()), Cursor::ResizeV);
        let mut turned = cx(&d, &s, &p);
        turned.screen = turned_view(90.0);
        assert_eq!(t.cursor(&turned, horizon, Mods::default()), Cursor::ResizeH);
        // Click the right face of the widget.
        let (faces, _) = g.widget(&d, 1.0);
        let q = faces[1].1;
        let centre = Point::new((q[0].x + q[1].x + q[2].x + q[3].x) / 4.0, (q[0].y + q[1].y + q[2].y + q[3].y) / 4.0);
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, centre.x, centre.y)),
            vec![Action::Exec("perspective.plane.set".into(), json!({"plane": "right"}))]
        );
        // Drag the left vanishing point.
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, g.vp_left, g.horizon));
        assert_eq!(acts, vec![Action::Begin("Edit Perspective Grid".into())]);
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, g.vp_left - 40.0, g.horizon + 10.0));
        let Action::Preview(cmd, v) = &acts[0] else { panic!() };
        assert_eq!(cmd, "perspective.grid.set");
        assert_eq!(v["vpLeft"], json!(g.vp_left - 40.0));
        assert_eq!(v["horizon"], json!(g.horizon + 10.0));
        assert!(!grid_overlays(&d, 1.0).is_empty());
    }

    #[test]
    fn depth_moves_along_the_plane_normal() {
        // A grid with a viewing angle foreshortens each axis by its own distance.
        let angled = PerspectiveGrid { angle: Some(30.0), ..grid(3) };
        assert_ne!(angled.foreshortening().0, angled.foreshortening().1);
        for g in [grid(1), grid(2), grid(3), angled] {
            let kind = g.kind;
            for pl in Plane::ALL {
                let q = Point::new(40.0, 30.0);
                let from = g.homography_at(pl, 0.0).unwrap().apply(q).unwrap();
                let to = g.homography_at(pl, 25.0).unwrap().apply(q).unwrap();
                let d = g.depth_at(pl, 0.0, from, to).unwrap();
                assert!((d - 25.0).abs() < 1e-6, "{kind} {pl:?} {d}");
                // The map to the parallel plane keeps plane coordinates.
                let m = g.transform_map(pl, 0.0, Affine::IDENTITY, 25.0).unwrap();
                assert!((m.apply(from).unwrap() - to).hypot() < 1e-6);
                // In-plane: a plane-space translation.
                let m = g.transform_map(pl, 25.0, Affine::translate((10.0, -5.0)), 0.0).unwrap();
                let moved = g.homography_at(pl, 25.0).unwrap().inverse().unwrap().apply(m.apply(to).unwrap()).unwrap();
                assert!((moved - Point::new(50.0, 25.0)).hypot() < 1e-6, "{kind} {pl:?} {moved:?}");
            }
        }
        // Behind the viewer there is no plane.
        assert!(grid(2).homography_at(Plane::Left, -1.0e9).is_none());
    }

    #[test]
    fn attachments_live_on_objects_and_legacy_ones_are_adopted() {
        let (mut d, id) = doc_with_rect();
        let mut g = grid(2);
        g.attached.insert(id.0.to_string(), Plane::Right);
        g.attached.insert("999999".into(), Plane::Left);
        // A grid as files from before stored it: the attachments in the grid.
        d.unknown.insert(DOC_KEY.into(), serde_json::to_value(&g).unwrap());
        let read = PerspectiveGrid::from_doc(&d).unwrap();
        assert_eq!(read.attachment_of(&d, id), Some((Plane::Right, 0.0)));
        read.store(&mut d);
        assert!(d.unknown[DOC_KEY].get("attached").is_none(), "the stored grid keeps none");
        assert_eq!(attachment(d.node(id).unwrap()), Some((Plane::Right, 0.0)));
        assert_eq!(PerspectiveGrid::from_doc(&d).unwrap().attached_plane(id), Some(Plane::Right));
        // Released since: not adopted.
        let (mut d2, id2) = doc_with_rect();
        let mut old = grid(2);
        old.attached.insert(id2.0.to_string(), Plane::Left);
        d2.unknown.insert(DOC_KEY.into(), serde_json::to_value(&old).unwrap());
        grid(2).store(&mut d2);
        assert!(d2.node(id2).unwrap().extras().perspective.is_none());
    }

    #[test]
    fn selection_tool_scales_with_handles_and_5_moves_perpendicular() {
        let (mut d, id) = doc_with_rect();
        grid(2).store(&mut d);
        set_attachment(d.node_mut(id).unwrap(), Plane::Right, 0.0);
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let c = cx(&d, &s, &p);
        let g = PerspectiveGrid::current(&d);
        let (plane, depth, rect) = g.plane_bounds(&d, &[id]).unwrap();
        assert_eq!((plane, depth), (Plane::Right, 0.0));
        let h = g.homography(Plane::Right).unwrap();
        let mut t = PerspectiveSelectionTool::default();
        assert!(t.claims_key(&c, ToolKey::Left));
        // A corner handle scales.
        let corner = h.apply(Point::new(rect.x1, rect.y1)).unwrap();
        assert!(matches!(t.cursor(&c, corner, Mods::default()), Cursor::ResizeNwSe | Cursor::ResizeNeSw));
        // In a view turned 90° its cursor turns with the view.
        let mut turned = cx(&d, &s, &p);
        turned.screen = turned_view(90.0);
        assert_eq!(t.cursor(&turned, corner, Mods::default()), t.cursor(&c, corner, Mods::default()).turned(90.0));
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, corner.x, corner.y)), vec![Action::Begin("Scale in Perspective".into())]);
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, corner.x + 10.0, corner.y + 10.0));
        assert!(matches!(&acts[0], Action::Preview(cmd, _) if cmd == "perspective.transform"));
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, corner.x + 10.0, corner.y + 10.0)), vec![Action::Commit]);
        // Moving: 5 toggles the perpendicular move.
        let mid = h.apply(rect.center()).unwrap();
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, mid.x, mid.y));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, mid.x + 20.0, mid.y));
        assert_eq!(acts[0], Action::Begin("Move in Perspective".into()));
        let Action::Preview(_, v) = &acts[1] else { panic!("{acts:?}") };
        assert_eq!(v["perpendicular"], json!(false));
        assert!(!t.claims_key(&c, ToolKey::Left), "busy");
        let acts = t.key(&c, ToolKey::Digit(5), Mods::default());
        let Action::Preview(cmd, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!((cmd.as_str(), &v["perpendicular"], &v["to"]), ("perspective.move", &json!(true), &json!([mid.x + 20.0, mid.y])));
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.starts_with("dZ"))));
        assert!(t.key(&c, ToolKey::Digit(3), Mods::default()).is_empty());
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, mid.x + 20.0, mid.y)), vec![Action::Commit]);
        assert!(!t.busy());
    }

    #[test]
    fn resize_cursors_follow_the_handle_on_screen() {
        assert_eq!(resize_cursor(Vec2::new(1.0, 0.1)), Cursor::ResizeH);
        assert_eq!(resize_cursor(Vec2::new(0.1, -1.0)), Cursor::ResizeV);
        assert_eq!(resize_cursor(Vec2::new(1.0, -1.0)), Cursor::ResizeNeSw);
        assert_eq!(resize_cursor(Vec2::new(1.0, 1.0)), Cursor::ResizeNwSe);
        assert_eq!(resize_cursor(Vec2::new(-1.0, -1.0)), Cursor::ResizeNwSe);
    }
}
