//! Measure tool: drag between two points to read distance, angle, dX and dY (Shift constrains to
//! 45°). The result stays on screen (and in the tool options, for agents) until the next drag; the
//! document is never changed. Escape during a drag cancels it: the result of the drag before
//! comes back.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Vec2};

use super::CYAN;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Default)]
pub struct MeasureTool {
    line: Option<(Point, Point)>,
    dragging: bool,
    /// The line measured before the drag under way, which Escape puts back.
    before: Option<(Point, Point)>,
}

/// (distance, angle in degrees counter-clockwise, dx, dy)
pub fn measure(a: Point, b: Point) -> (f64, f64, f64, f64) {
    let d = b - a;
    (d.hypot(), (-d.y).atan2(d.x).to_degrees() + 0.0, d.x, d.y)
}

impl Tool for MeasureTool {
    fn id(&self) -> &'static str {
        "measure"
    }

    fn busy(&self) -> bool {
        self.dragging
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                let (p, _) = crate::guides::snap_draw(cx, ev.pos, &[]);
                self.before = self.line;
                self.line = Some((p, p));
                self.dragging = true;
            }
            PointerKind::Drag | PointerKind::Up if self.dragging => {
                if let Some((a, _)) = self.line {
                    let b = if ev.mods.shift {
                        a + vectorcraft_geom::constrain_angle_from(ev.pos - a, 45.0, cx.constrain_angle)
                    } else {
                        crate::guides::snap_draw(cx, ev.pos, &[]).0
                    };
                    self.line = Some((a, b));
                }
                self.dragging = ev.kind == PointerKind::Drag;
            }
            _ => {}
        }
        vec![]
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.dragging {
            self.dragging = false;
            self.line = self.before;
        }
        vec![]
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some((a, b)) = self.line else { return vec![] };
        if a == b {
            return vec![];
        }
        let mut out = vec![Overlay::Line { a, b, color: CYAN, dashed: false }];
        // Measurement Labels decides the readout; the line the tool draws stays.
        if cx.measurement_labels {
            let (dist, ang, dx, dy) = measure(a, b);
            out.push(Overlay::Measure {
                p: b + Vec2::new(cx.tol(12.0), cx.tol(12.0)),
                text: format!("D: {}\nAngle: {ang:.1}°\n{}", cx.len(dist), cx.offset_label(dx, dy)),
            });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Measure
    }

    fn options(&self) -> Value {
        match self.line {
            Some((a, b)) => {
                let (d, ang, dx, dy) = measure(a, b);
                json!({ "x": a.x, "y": a.y, "distance": d, "angle": ang, "dx": dx, "dy": dy })
            }
            None => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn horizontal_drag_reports_positive_zero_angle() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = MeasureTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 40.0, 10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 40.0, 10.0));
        let ang = t.options()["angle"].as_f64().unwrap();
        assert!(ang == 0.0 && ang.is_sign_positive(), "angle was {ang:?}");
        let ov = t.overlays(&cx);
        assert!(matches!(&ov[1], Overlay::Measure { text, .. } if text.contains("Angle: 0.0°")));
    }

    #[test]
    fn drag_reports_distance_and_angle() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = MeasureTool::default();
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0)).is_empty());
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 40.0, -30.0)).is_empty());
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 40.0, -30.0)).is_empty());
        let o = t.options();
        assert!((o["distance"].as_f64().unwrap() - 50.0).abs() < 1e-9);
        assert!((o["angle"].as_f64().unwrap() - 53.130102).abs() < 1e-4);
        let ov = t.overlays(&cx);
        assert!(matches!(&ov[1], Overlay::Measure { text, .. } if text.starts_with("D: 50.00 pt")));
        let quiet = ToolContext { measurement_labels: false, ..cx };
        let ov = t.overlays(&quiet);
        assert!(!ov.iter().any(|o| matches!(o, Overlay::Measure { .. })), "Measurement Labels off hides the readout");
        assert!(ov.iter().any(|o| matches!(o, Overlay::Line { .. })), "the measured line stays");
    }

    /// Escape during a drag ends it and brings back the result of the drag before (none at
    /// first); the release after it measures nothing.
    #[test]
    fn escape_cancels_a_measure_drag() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = MeasureTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 40.0, -30.0));
        assert!(t.key(&cx, ToolKey::Escape, Mods::default()).is_empty());
        assert!(!t.busy());
        assert_eq!(t.options(), Value::Null, "no earlier result");
        assert!(t.overlays(&cx).is_empty());
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 40.0, -30.0));
        assert_eq!(t.options(), Value::Null, "the release measures nothing");
        // A measurement, then a second drag canceled: the first one's result is back.
        for (kind, x, y) in [(PointerKind::Down, 10.0, 10.0), (PointerKind::Drag, 40.0, -30.0), (PointerKind::Up, 40.0, -30.0)] {
            t.pointer(&cx, &PointerEvent::new(kind, x, y));
        }
        let first = t.options();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 100.0));
        assert!(t.key(&cx, ToolKey::Enter, Mods::default()).is_empty());
        assert!(t.busy(), "Enter leaves the drag alone");
        t.key(&cx, ToolKey::Escape, Mods::default());
        assert!(!t.busy());
        assert_eq!(t.options(), first);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 130.0, 100.0));
        assert_eq!(t.options(), first);
        assert!((t.options()["distance"].as_f64().unwrap() - 50.0).abs() < 1e-9);
    }
}
