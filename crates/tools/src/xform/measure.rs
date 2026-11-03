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

/// The end of a measurement from `a` with the pointer at `p`: `p` snapped as a drawing tool snaps
/// it. With Shift held, the end stays on the 45° step from the Constrain Angle nearest the
/// pointer's direction: `p` is projected onto the step, then snapped along it as the Line Segment
/// tool snaps a Shift-drawn end. With Snap to Point alone, the point `p` snaps to (an anchor, or
/// in line with a ruler guide) is projected instead. When the snapped point lies nearer another
/// step (the grid rounds it past the halfway angle), it is turned back onto the pointer's step.
pub(crate) fn end_point(cx: &ToolContext, a: Point, p: Point, shift: bool) -> Point {
    if !shift {
        return crate::guides::snap_draw(cx, p, &[]).0;
    }
    let d = vectorcraft_geom::constrain_angle_from(p - a, 45.0, cx.constrain_angle);
    let len = d.hypot();
    if len < 1e-12 {
        return a;
    }
    let u = d / len;
    let to_point = cx.snap_to_point && !(cx.smart_guides || cx.snap_to_grid || cx.snap_to_pixel);
    let s = if to_point { crate::guides::snap_draw(cx, p, &[]).0 } else { p };
    let q = a + u * (s - a).dot(u);
    let leave = crate::guides::Leave::segment(cx, a, true);
    let r = crate::guides::PointSnap::new(cx, || crate::guides::Targets::collect(cx.doc, &[], None)).snap_from(cx, q, Some(&leave)).0;
    let v = r - a;
    if v.cross(u).abs() <= 1e-9 * v.hypot().max(1.0) { r } else { a + u * v.hypot() }
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
                    self.line = Some((a, end_point(cx, a, ev.pos, ev.mods.shift)));
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

    /// A Shift-drag snaps its end point as a drag without Shift does, then projects it onto its
    /// 45° step: onto the grid with Snap to Grid, in line with an anchor with Smart Guides. With
    /// no snapping, the end is the pointer projected onto the step.
    #[test]
    fn shift_drag_snaps_its_end_point() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        cx.snap_to_grid = true;
        let shift = Mods { shift: true, ..Mods::default() };
        let mut t = MeasureTool::default();
        // (37.3, 12.1) snaps to (36, 9) on the 9 pt grid, and Shift keeps it at 0°: (36, 0).
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 37.3, 12.1).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 37.3, 12.1).with_mods(shift));
        let o = t.options();
        assert!((o["distance"].as_f64().unwrap() - 36.0).abs() < 1e-9, "on the 9 pt grid: {o}");
        assert!(o["dy"].as_f64().unwrap().abs() < 1e-9, "kept horizontal: {o}");
        // Near 45°: (31.4, 27.7) snaps to (27, 27).
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 31.4, 27.7).with_mods(shift));
        let o = t.options();
        assert!((o["dx"].as_f64().unwrap() - 27.0).abs() < 1e-9, "{o}");
        assert!((o["dy"].as_f64().unwrap() - 27.0).abs() < 1e-9, "{o}");
        // No snapping: the pointer is projected onto 0°, so the measurement is 37.3 pt.
        cx.snap_to_grid = false;
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 37.3, 12.1).with_mods(shift));
        let o = t.options();
        assert!((o["distance"].as_f64().unwrap() - 37.3).abs() < 1e-9, "{o}");
        assert!(o["dy"].as_f64().unwrap().abs() < 1e-9, "{o}");
        // Snap to Point alone: the end snaps to the anchor at (200, 200) and is projected onto
        // 0°, so it ends in line with it: (200, 203).
        assert!(cx.snap_to_point);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 203.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 201.0, 201.0).with_mods(shift));
        let o = t.options();
        assert!((o["dx"].as_f64().unwrap() + 100.0).abs() < 1e-9, "{o}");
        assert!(o["dy"].as_f64().unwrap().abs() < 1e-9, "{o}");
        // An anchor on the 45° step: the end lands on it.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 101.0, 100.0).with_mods(shift));
        let o = t.options();
        assert!((o["dx"].as_f64().unwrap() - 100.0).abs() < 1e-9, "{o}");
        assert!((o["dy"].as_f64().unwrap() - 100.0).abs() < 1e-9, "{o}");
        // Smart Guides: from (300, 300), Shift keeps the end at 0° and it lands in line with x = 200.
        cx.snap_to_grid = false;
        cx.smart_guides = true;
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 203.0, 303.0).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 203.0, 303.0).with_mods(shift));
        let o = t.options();
        assert!((o["distance"].as_f64().unwrap() - 100.0).abs() < 1e-9, "{o}");
        assert!(o["dy"].as_f64().unwrap().abs() < 1e-9, "{o}");
        // At 45°, the end slides along the step to where it meets x = 200: (200, 220).
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 320.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 203.0, 222.5).with_mods(shift));
        let o = t.options();
        assert!((o["dx"].as_f64().unwrap() + 100.0).abs() < 1e-9, "{o}");
        assert!((o["dy"].as_f64().unwrap() + 100.0).abs() < 1e-9, "{o}");
    }

    /// With a Constrain Angle of 30°, the grid point a Shift-drag's end rounds to can lie nearer
    /// another step; the end stays on the step nearest the pointer's direction.
    #[test]
    fn shift_drag_keeps_its_step_after_the_grid_rounds_it() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        cx.snap_to_grid = true;
        cx.constrain_angle = 30.0;
        let shift = Mods { shift: true, ..Mods::default() };
        let mut t = MeasureTool::default();
        // (5, -3) points 31° up, so the step is 30°; the end lands at about (5.05, -2.92), which
        // the 9 pt grid rounds to (9, 0), nearer the -15° step.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 5.0, -3.0).with_mods(shift));
        let o = t.options();
        assert!((o["angle"].as_f64().unwrap() - 30.0).abs() < 1e-9, "{o}");
    }

    /// With Snap to Point alone, a Shift-drag's end slides along its 45° step to a ruler guide.
    #[test]
    fn shift_drag_meets_a_ruler_guide() {
        let (mut d, _) = doc_with_rect();
        d.guides.push(vectorcraft_doc::Guide::new(true, 50.0));
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        assert!(cx.snap_to_point);
        let shift = Mods { shift: true, ..Mods::default() };
        let mut t = MeasureTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 51.0, 49.0).with_mods(shift));
        let o = t.options();
        assert!((o["dx"].as_f64().unwrap() - 50.0).abs() < 1e-9, "{o}");
        assert!((o["dy"].as_f64().unwrap() - 50.0).abs() < 1e-9, "{o}");
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
