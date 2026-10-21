//! Shape tools: Rectangle, Rounded Rectangle, Ellipse, Polygon, Star, Line Segment.
//!
//! Drag draws (Shift constrains to square/circle/45°, Alt draws from the centre, Space held moves
//! the shape being drawn); a click without a drag asks the UI for the size dialog (like
//! Illustrator). ↑/↓ during a polygon/star drag change the side/point count; during a star drag,
//! Ctrl/Cmd keeps the inner radius where it is while the outer one follows the pointer, and Alt
//! keeps the star's shoulders straight (#812). The start point and
//! the dragged corner snap to Smart Guides ([`DrawSnap`]), hovering before the press too.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::guides::{DrawSnap, Leave, square};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

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
            _ => Some(Leave::diagonal(start, m.shift)),
        }
    }

    /// The command for a drag from `start` to `p` (snapped: a line's end already keeps Shift's
    /// angle).
    pub fn command(&self, start: Point, p: Point, m: Mods) -> (String, Value) {
        match self.id {
            "polygon" | "star" => {
                let v = p - start;
                let r = v.hypot().max(0.01);
                let rot = if m.shift { 0.0 } else { (v.atan2().to_degrees() + 90.0) % 360.0 };
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
                ("shape.line".into(), json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y }))
            }
            _ => {
                let r = drag_rect(start, p, m);
                let cmd = if self.id == "ellipse" { "shape.ellipse" } else { "shape.rectangle" };
                let mut v = json!({ "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height() });
                if self.id == "roundedRectangle" {
                    v["radius"] = json!(self.corner_radius);
                }
                (cmd.into(), v)
            }
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
                let (c, v) = self.command(s, ev.pos, ev.mods);
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
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
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
                self.corner_radius += 1.0;
                true
            }
            ("roundedRectangle", ToolKey::Down) => {
                self.corner_radius = (self.corner_radius - 1.0).max(0.0);
                true
            }
            _ => false,
        };
        if changed {
            let (c, v) = self.command(s, self.last, self.mods);
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
                let (w, h) = match self.id {
                    "polygon" | "star" => ((self.last - s).x.abs(), (self.last - s).y.abs()),
                    _ => {
                        let r = drag_rect(s, self.last, self.mods);
                        (r.width(), r.height())
                    }
                };
                cx.size_label(w, h)
            };
            o.push(Overlay::Measure { p, text });
        }
        o
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        json!({ "sides": self.sides, "points": self.points, "cornerRadius": self.corner_radius, "starRatio": self.star_ratio })
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "sides" => self.sides = v.as_u64().unwrap_or(6).clamp(3, 1000) as u32,
            "points" => self.points = v.as_u64().unwrap_or(5).clamp(3, 1000) as u32,
            "cornerRadius" => self.corner_radius = v.as_f64().unwrap_or(12.0).max(0.0),
            "starRatio" => self.star_ratio = v.as_f64().unwrap_or(0.5).clamp(0.01, 1.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn rect_drag() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeTool::new("rectangle");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0, 30.0));
        assert_eq!(a[0], Action::Begin("Rectangle".into()));
        assert_eq!(a[1], Action::Preview("shape.rectangle".into(), json!({"x": 10.0, "y": 10.0, "width": 40.0, "height": 20.0})));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 50.0, 30.0)), vec![Action::Commit]);
    }

    #[test]
    fn space_moves_the_shape_being_drawn() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let space = Mods { space: true, ..Default::default() };
        let mut t = ShapeTool::new("rectangle");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 120.0));
        // Space held: the 100 × 70 rectangle follows the pointer.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 170.0).with_mods(space));
        assert_eq!(a.last(), Some(&Action::Preview("shape.rectangle".into(), json!({"x": 150.0, "y": 100.0, "width": 100.0, "height": 70.0}))));
        // Let go of Space: it grows again, from where it was moved to.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 270.0, 190.0));
        assert_eq!(a.last(), Some(&Action::Preview("shape.rectangle".into(), json!({"x": 150.0, "y": 100.0, "width": 120.0, "height": 90.0}))));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 270.0, 190.0)), vec![Action::Commit]);
        // A star (drawn from its centre) moves its centre.
        let mut t = ShapeTool::new("star");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 50.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 60.0).with_mods(space));
        assert!(matches!(a.last(), Some(Action::Preview(_, v)) if v["cx"] == 60.0 && v["cy"] == 60.0), "{a:?}");
    }

    /// During a star drag, Ctrl/Cmd keeps the inner radius while the outer one follows the pointer
    /// (the new ratio stays once the key is let go), and Alt keeps the shoulders straight (#812).
    #[test]
    fn a_star_drag_holds_its_inner_radius_with_ctrl_and_straightens_with_alt() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let radii = |a: &[Action]| match a.last() {
            Some(Action::Preview(_, v)) => (v["radius1"].as_f64().unwrap(), v["radius2"].as_f64().unwrap()),
            other => panic!("{other:?}"),
        };
        let ctrl = Mods { cmd: true, ..Default::default() };
        let mut t = ShapeTool::new("star");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
        let (r1, r2) = radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 50.0)));
        assert_eq!((r1, r2), (40.0, 20.0));
        // Ctrl: the inner radius stays 20 as the outer one grows to 80.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 50.0).with_mods(ctrl));
        let (r1, r2) = radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 50.0).with_mods(ctrl)));
        assert!((r1 - 80.0).abs() < 1e-9 && (r2 - 20.0).abs() < 1e-9, "{r1} {r2}");
        // Let go: the ratio (a quarter) stays.
        let (r1, r2) = radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 50.0)));
        assert!((r2 / r1 - 0.25).abs() < 1e-9, "{r1} {r2}");
        // Alt: straight shoulders, about 0.382 for five points.
        let alt = Mods { alt: true, ..Default::default() };
        let (r1, r2) = radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 50.0).with_mods(alt)));
        assert!((r2 / r1 - 0.381_966).abs() < 1e-5, "{}", r2 / r1);
        assert_eq!(straight_ratio(4), None, "four points can't line up");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 50.0)), vec![Action::Commit]);
    }

    #[test]
    fn shift_square_alt_center() {
        let t = ShapeTool::new("ellipse");
        let (c, v) = t.command(Point::new(0.0, 0.0), Point::new(10.0, 4.0), Mods { shift: true, alt: true, ..Default::default() });
        assert_eq!(c, "shape.ellipse");
        assert_eq!(v, json!({"x": -10.0, "y": -10.0, "width": 20.0, "height": 20.0}));
    }

    #[test]
    fn click_opens_dialog() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeTool::new("star");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 10.0)),
            vec![Action::Dialog("star".into(), json!({"x": 10.0, "y": 10.0}))]
        );
    }

    #[test]
    fn alt_click_passes_draw_from_centre_to_the_dialog() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Default::default() };
        let mut t = ShapeTool::new("rectangle");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0).with_mods(alt));
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 10.0).with_mods(alt)),
            vec![Action::Dialog("rectangle".into(), json!({"x": 10.0, "y": 10.0, "alt": true}))]
        );
    }

    #[test]
    fn arrow_keys_change_sides() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeTool::new("polygon");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0, 10.0));
        let a = t.key(&cx, ToolKey::Up, Mods::default());
        assert!(matches!(&a[0], Action::Preview(_, v) if v["sides"] == 7));
    }

    /// The start and the dragged corner land on another object's anchors (#506), hovering shows
    /// it before the press, and the measurement label gives the size drawn.
    #[test]
    fn corners_snap_to_smart_guides() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeTool::new("rectangle");
        let anchor = |o: &[Overlay], at: Point| o.iter().any(|o| matches!(o, Overlay::Label { text, p, .. } if text == "anchor" && *p == at));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 98.0, 102.0));
        assert!(anchor(&t.overlays(&cx), Point::new(100.0, 100.0)));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 98.0, 102.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 198.0, 202.0));
        assert_eq!(a[1], Action::Preview("shape.rectangle".into(), json!({"x": 100.0, "y": 100.0, "width": 100.0, "height": 100.0})));
        let o = t.overlays(&cx);
        assert!(anchor(&o, Point::new(200.0, 200.0)), "{o:?}");
        assert!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.size_label(100.0, 100.0))));
        // Alt draws from the centre: the label gives the whole size.
        let alt = Mods { alt: true, ..Mods::default() };
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 198.0, 202.0).with_mods(alt));
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.size_label(200.0, 200.0))));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 198.0, 202.0));
        assert!(t.overlays(&cx).is_empty());
        // A line with Shift keeps to 45° steps, sliding into line with the rect's right side.
        let mut t = ShapeTool::new("lineSegment");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        let shift = Mods { shift: true, ..Mods::default() };
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 203.0, 302.0).with_mods(shift));
        assert!(matches!(&a[..], [_, Action::Preview(_, v)] if v["x2"] == 200.0 && (v["y2"].as_f64().unwrap() - 300.0).abs() < 1e-9), "{a:?}");
        let from = Point::new(300.0, 300.0);
        let to = Point::new(200.0, 300.0);
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.segment_measure_label(from, to))));
    }
}
