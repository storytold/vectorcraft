//! Shape tools: Rectangle, Rounded Rectangle, Ellipse, Polygon, Star, Line Segment.
//!
//! Drag draws (Shift constrains to square/circle/45°, Alt draws from the centre, Space held moves
//! the shape being drawn); a click without a drag asks the UI for the size dialog (like
//! Illustrator). ↑/↓ during a polygon/star drag change the side/point count; during a star drag,
//! Ctrl/Cmd keeps the inner radius where it is while the outer one follows the pointer, and Alt
//! keeps the star's shoulders straight (#812). The start point and
//! the dragged corner snap to Smart Guides ([`DrawSnap`]), hovering before the press too.

use serde_json::{Value, json};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::guides::{DrawSnap, Leave, square};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The largest corner radius the Rounded Rectangle tool draws, points: the range of General ›
/// Corner Radius, which the tool's radius is (its `prefs.list` spec), so ↑ and a Tool Options
/// value stop there too.
pub const MAX_CORNER_RADIUS: f64 = 1296.0;

pub struct ShapeTool {
    id: &'static str,
    start: Option<Point>,
    last: Point,
    mods: Mods,
    began: bool,
    pub sides: u32,
    pub points: u32,
    pub corner_radius: f64,
    /// Star inner radius as a fraction of the outer radius.
    pub star_ratio: f64,
    /// The star's inner radius held while Ctrl/Cmd is down in a drag.
    inner: Option<f64>,
    /// Line Segment Tool Options: the length and angle (degrees) a click's dialog opens with, and
    /// Fill Line (new lines take the current fill).
    pub line_length: f64,
    pub line_angle: f64,
    pub fill_line: bool,
    snap: DrawSnap,
}

/// The inner-to-outer radius ratio that keeps a star of `points` points' shoulders straight (each
/// point's sides in line with the next points' sides: a regular star polygon {n/2}); none for
/// fewer than five points, whose shoulders can't line up.
fn straight_ratio(points: u32) -> Option<f64> {
    let n = f64::from(points);
    (points >= 5).then(|| (2.0 * std::f64::consts::PI / n).cos() / (std::f64::consts::PI / n).cos())
}

impl ShapeTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "roundedRectangle" => "roundedRectangle",
            "ellipse" => "ellipse",
            "polygon" => "polygon",
            "star" => "star",
            "lineSegment" => "lineSegment",
            _ => "rectangle",
        };
        Self {
            id,
            start: None,
            last: Point::ZERO,
            mods: Mods::default(),
            began: false,
            sides: 6,
            points: 5,
            corner_radius: 12.0,
            star_ratio: 0.5,
            inner: None,
            snap: DrawSnap::default(),
            line_length: 100.0,
            line_angle: 0.0,
            fill_line: false,
        }
    }

    /// Ctrl/Cmd in a star drag from `start` to `p`: the inner radius stays as it was when the key
    /// went down, so the ratio follows the outer radius (and stays so once the key is let go).
    fn hold_inner(&mut self, start: Point, p: Point, m: Mods) {
        let outer = (p - start).hypot();
        match (m.cmd, self.inner) {
            (true, None) => self.inner = Some(outer * self.star_ratio),
            (false, Some(_)) => self.inner = None,
            _ => {}
        }
        if let Some(inner) = self.inner
            && outer > 0.0
            && (inner / outer).is_finite()
        {
            self.star_ratio = (inner / outer).clamp(0.01, 1.0);
        }
    }

    fn label(&self) -> &'static str {
        match self.id {
            "roundedRectangle" => "Rounded Rectangle",
            "ellipse" => "Ellipse",
            "polygon" => "Polygon",
            "star" => "Star",
            "lineSegment" => "Line",
            _ => "Rectangle",
        }
    }

    /// How the dragged point keeps to the start: a polygon's or star's radius goes anywhere, a
    /// line is a segment (Shift: 45° steps), the rest are boxes (Shift: a square).
    fn leave(&self, cx: &ToolContext, start: Point, m: Mods) -> Option<Leave> {
        match self.id {
            "polygon" | "star" => None,
            "lineSegment" => Some(Leave::segment(cx, start, m.shift)),
            // With a Constrain Angle, Shift's square is taken along the rotated axes
            // ([`Self::command_at`]), not the page's diagonal.
            _ => Some(Leave::diagonal(start, m.shift && vectorcraft_geom::constrain_axes(cx.constrain_angle).is_none())),
        }
    }

    /// The command for a drag from `start` to `p` (snapped: a line's end already keeps Shift's
    /// angle).
    pub fn command(&self, start: Point, p: Point, m: Mods) -> (String, Value) {
        self.command_at(start, p, m, 0.0)
    }

    /// [`Self::command`] with General › Constrain Angle `base` (degrees, counter-clockwise):
    /// rectangles, rounded rectangles and ellipses are drawn along the x and y axes turned by it,
    /// as the reference app draws new objects along the rotated axes
    /// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/arrange-objects/rotate-objects-by-specific-angles.html>).
    pub fn command_at(&self, start: Point, p: Point, m: Mods, base: f64) -> (String, Value) {
        match self.id {
            "polygon" | "star" => {
                let v = p - start;
                let r = v.hypot().max(0.01);
                // Shift keeps the base orientation: upright, turned by the Constrain Angle.
                let rot = if m.shift { vectorcraft_geom::constrain_rotation(base).unwrap_or(0.0) } else { (v.atan2().to_degrees() + 90.0) % 360.0 };
                if self.id == "polygon" {
                    ("shape.polygon".into(), json!({ "cx": start.x, "cy": start.y, "radius": r, "sides": self.sides, "rotation": rot }))
                } else {
                    let ratio = if m.alt { straight_ratio(self.points).unwrap_or(self.star_ratio) } else { self.star_ratio };
                    (
                        "shape.star".into(),
                        json!({ "cx": start.x, "cy": start.y, "radius1": r, "radius2": r * ratio, "points": self.points, "rotation": rot }),
                    )
                }
            }
            "lineSegment" => {
                let (a, b) = if m.alt { (start - (p - start), p) } else { (start, p) };
                let mut v = json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y });
                if self.fill_line {
                    v["fill"] = json!(true);
                }
                ("shape.line".into(), v)
            }
            _ => {
                let cmd = if self.id == "ellipse" { "shape.ellipse" } else { "shape.rectangle" };
                let mut v = box_params(start, p, m, base);
                if self.id == "roundedRectangle" {
                    v["radius"] = json!(self.corner_radius);
                }
                (cmd.into(), v)
            }
        }
    }
}

/// The box a drag from `start` to `p` draws (from its centre with Alt, square with Shift),
/// measured along the page axes turned by Constrain Angle `base`: the box in those axes, and the
/// axes (`None` for the page's own: 0° or a whole turn, where the box is on the page exactly).
/// The drawn shape and its size label both come from it.
pub(crate) fn turned_box(start: Point, p: Point, m: Mods, base: f64) -> (Rect, Option<Affine>) {
    match vectorcraft_geom::constrain_axes(base) {
        None => (drag_rect(start, p, m), None),
        Some(axes) => {
            let back = axes.inverse();
            (drag_rect(back * start, back * p, m), Some(axes))
        }
    }
}

/// The `{x, y, width, height, rotation?}` params of the box a drag from `start` to `p` draws along
/// Constrain Angle `base` ([`turned_box`]): the box's page position at 0° or a whole turn, else
/// the turned box's centre out on the page again with the rotation that turns it about it.
pub(crate) fn box_params(start: Point, p: Point, m: Mods, base: f64) -> Value {
    match turned_box(start, p, m, base) {
        (r, None) => json!({ "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height() }),
        (r, Some(axes)) => {
            let c = axes * r.center();
            let rotation = -vectorcraft_geom::normalize_deg(base);
            json!({ "x": c.x - r.width() / 2.0, "y": c.y - r.height() / 2.0, "width": r.width(), "height": r.height(), "rotation": rotation })
        }
    }
}

/// Space held once a shape drag has begun moves the shape being drawn instead of sizing it: `start`
/// moves as far as the pointer did since `last`, which becomes the pointer.
pub(crate) fn space_moves(start: &mut Point, last: &mut Point, ev: &PointerEvent, began: bool) {
    if ev.mods.space && began {
        *start += ev.pos - *last;
    }
    *last = ev.pos;
}

/// The rectangle a drag from `start` to `p` draws: Shift makes it a square, Alt draws it from its
/// centre.
pub(crate) fn drag_rect(start: Point, p: Point, m: Mods) -> Rect {
    let d = if m.shift { square(p - start) } else { p - start };
    if m.alt { Rect::from_points(start - d, start + d) } else { Rect::from_points(start, start + d) }
}

impl Tool for ShapeTool {
    fn id(&self) -> &'static str {
        self.id
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
                let pos = self.snap.drag(cx, ev.pos, self.leave(cx, s, ev.mods).as_ref());
                let ev = &PointerEvent { pos, ..*ev };
                space_moves(&mut s, &mut self.last, ev, self.began);
                self.start = Some(s);
                self.mods = ev.mods;
                if self.id == "star" {
                    self.hold_inner(s, ev.pos, ev.mods);
                }
                let mut out = vec![];
                if !self.began {
                    if ev.pos.distance(s) < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    out.push(Action::Begin(self.label().into()));
                }
                let (c, v) = self.command_at(s, ev.pos, ev.mods, cx.constrain_angle);
                out.push(Action::Preview(c, v));
                out
            }
            PointerKind::Up => {
                let Some(s) = self.start.take() else { return vec![] };
                self.snap.clear();
                self.inner = None;
                if self.began {
                    self.began = false;
                    vec![Action::Commit]
                } else {
                    // Alt (draw from centre) is passed on, so the size dialog can centre the shape on the click.
                    let args = if ev.mods.alt { json!({ "x": s.x, "y": s.y, "alt": true }) } else { json!({ "x": s.x, "y": s.y }) };
                    vec![Action::Dialog(self.id.into(), args)]
                }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        let Some(s) = self.start.filter(|_| self.began) else { return vec![] };
        let changed = match (self.id, key) {
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
            ("roundedRectangle", ToolKey::Up) => {
                self.corner_radius = (self.corner_radius + 1.0).min(MAX_CORNER_RADIUS);
                true
            }
            ("roundedRectangle", ToolKey::Down) => {
                self.corner_radius = (self.corner_radius - 1.0).max(0.0);
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
            let text = if self.id == "lineSegment" {
                cx.segment_measure_label(s, self.last)
            } else {
                // A box's size as drawn (from its centre with Alt, square with Shift).
                match self.id {
                    "star" => {
                        let outer = (self.last - s).hypot().max(0.01);
                        // The same ratio `command_at` draws with, so the label matches the preview
                        // (Ctrl/Cmd's held radius is already in `star_ratio`, clamped; Alt overrides it).
                        let ratio = if self.mods.alt { straight_ratio(self.points).unwrap_or(self.star_ratio) } else { self.star_ratio };
                        cx.star_radii_label(outer, outer * ratio)
                    }
                    _ => {
                        let (w, h) = match self.id {
                            "polygon" => ((self.last - s).x.abs(), (self.last - s).y.abs()),
                            _ => {
                                let (r, _) = turned_box(s, self.last, self.mods, cx.constrain_angle);
                                (r.width(), r.height())
                            }
                        };
                        cx.size_label(w, h)
                    }
                }
            };
            o.push(Overlay::Measure { p, text });
        }
        o
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        json!({
            "sides": self.sides,
            "points": self.points,
            "cornerRadius": self.corner_radius,
            "starRatio": self.star_ratio,
            "length": self.line_length,
            "angle": self.line_angle,
            "fillLine": self.fill_line,
        })
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        let finite = v.as_f64().filter(|n| n.is_finite());
        match key {
            "sides" => self.sides = v.as_u64().unwrap_or(6).clamp(3, 1000) as u32,
            "points" => self.points = v.as_u64().unwrap_or(5).clamp(3, 1000) as u32,
            "cornerRadius" => self.corner_radius = finite.unwrap_or(12.0).clamp(0.0, MAX_CORNER_RADIUS),
            "starRatio" => self.star_ratio = v.as_f64().unwrap_or(0.5).clamp(0.01, 1.0),
            "length" => self.line_length = finite.map_or(self.line_length, |n| n.abs().min(1e6)),
            "angle" => self.line_angle = finite.map_or(self.line_angle, |n| n.rem_euclid(360.0)),
            "fillLine" => self.fill_line = v.as_bool().unwrap_or(self.fill_line),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
