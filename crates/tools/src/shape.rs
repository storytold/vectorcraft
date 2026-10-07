//! Shape tools: Rectangle, Rounded Rectangle, Ellipse, Polygon, Star, Line Segment.
//!
//! Drag draws (Shift constrains to square/circle/45°, Alt draws from the centre); a click without a
//! drag asks the UI for the size dialog (like Illustrator). ↑/↓ during a polygon/star drag change the
//! side/point count.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect};

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
    guides: Vec<Overlay>,
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
            guides: vec![],
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

    /// The command for the current drag.
    pub fn command(&self, start: Point, p: Point, m: Mods) -> (String, Value) {
        match self.id {
            "polygon" | "star" => {
                let v = p - start;
                let r = v.hypot().max(0.01);
                let rot = if m.shift { 0.0 } else { (v.atan2().to_degrees() + 90.0) % 360.0 };
                if self.id == "polygon" {
                    ("shape.polygon".into(), json!({ "cx": start.x, "cy": start.y, "radius": r, "sides": self.sides, "rotation": rot }))
                } else {
                    (
                        "shape.star".into(),
                        json!({ "cx": start.x, "cy": start.y, "radius1": r, "radius2": r * self.star_ratio, "points": self.points, "rotation": rot }),
                    )
                }
            }
            "lineSegment" => {
                let (a, b) = if m.alt { (start - (p - start), p) } else { (start, p) };
                let b = if m.shift { a + vectorcraft_geom::constrain_angle(b - a, 45.0) } else { b };
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

/// The rectangle a drag from `start` to `p` draws: Shift makes it a square, Alt draws it from its
/// centre.
pub(crate) fn drag_rect(start: Point, p: Point, m: Mods) -> Rect {
    let mut d = p - start;
    if m.shift {
        let s = d.x.abs().max(d.y.abs());
        d = vectorcraft_geom::Vec2::new(s * d.x.signum(), s * d.y.signum());
    }
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
            // Hovering shows where a click would snap (the guide, its label and a marker).
            PointerKind::Move if self.start.is_none() => {
                self.guides = crate::guides::snap_draw(cx, ev.pos, &[]).1;
                vec![]
            }
            PointerKind::Down => {
                let (p, g) = crate::guides::snap_draw(cx, ev.pos, &[]);
                self.guides = g;
                self.start = Some(p);
                self.last = ev.pos;
                self.began = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some(s) = self.start else { return vec![] };
                // A line snaps tangent or perpendicular to the paths it meets.
                let from = (self.id == "lineSegment").then_some(s);
                let (pos, g) = crate::guides::snap_draw_from(cx, ev.pos, &[], from);
                self.guides = g;
                let ev = &PointerEvent { pos, ..*ev };
                self.last = ev.pos;
                self.mods = ev.mods;
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
                self.guides.clear();
                if self.began {
                    self.began = false;
                    vec![Action::Commit]
                } else {
                    vec![Action::Dialog(self.id.into(), json!({ "x": s.x, "y": s.y }))]
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
            vec![Action::Cancel]
        } else {
            vec![]
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        match self.start {
            Some(s) if self.began => {
                let d = self.last - s;
                let mut o = self.guides.clone();
                o.push(Overlay::Measure { p: self.last, text: cx.size_label(d.x.abs(), d.y.abs()) });
                o
            }
            _ => self.guides.clone(),
        }
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
}
