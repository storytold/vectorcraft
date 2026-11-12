//! Measure tool: drag between two points to read distance, angle, dX and dY (Shift constrains to
//! 45°). The result stays on screen (and in the tool options, for agents) until the next drag; the
//! document is never changed. Escape during a drag cancels it: the result of the drag before
//! comes back.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Vec2};

use super::CYAN;
use crate::guides::DrawSnap;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Default)]
pub struct MeasureTool {
    line: Option<(Point, Point)>,
    dragging: bool,
    /// The line measured before the drag under way, which Escape puts back.
    before: Option<(Point, Point)>,
    /// Snapping for both ends, its targets kept while nothing they depend on changes.
    snap: DrawSnap,
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
/// step (the grid rounds it past the halfway angle) or behind the start, it is turned back onto
/// the pointer's step. With View › Snap to Glyph on, the guides of live type are targets too.
pub(crate) fn end_point(cx: &ToolContext, snap: &mut DrawSnap, a: Point, p: Point, shift: bool) -> Point {
    if !shift {
        return snap.measure(cx, p, None);
    }
    let d = vectorcraft_geom::constrain_angle_from(p - a, 45.0, cx.constrain_angle);
    let len = d.hypot();
    if len < 1e-12 {
        return a;
    }
    let u = d / len;
    let to_point = cx.snap_to_point && !(cx.smart_guides || cx.snap_to_grid || cx.snap_to_pixel);
    let s = if to_point { snap.measure(cx, p, None) } else { p };
    let q = a + u * (s - a).dot(u);
    let leave = crate::guides::Leave::segment(cx, a, true);
    let r = snap.measure(cx, q, Some(&leave));
    let v = r - a;
    if v.cross(u).abs() <= 1e-9 * v.hypot().max(1.0) && v.dot(u) >= 0.0 { r } else { a + u * v.hypot() }
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
                let p = self.snap.measure(cx, ev.pos, None);
                self.before = self.line;
                self.line = Some((p, p));
                self.dragging = true;
            }
            PointerKind::Drag | PointerKind::Up if self.dragging => {
                if let Some((a, _)) = self.line {
                    self.line = Some((a, end_point(cx, &mut self.snap, a, ev.pos, ev.mods.shift)));
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
        let guides = self.snap.guides().iter().filter(|_| self.dragging).cloned();
        let Some((a, b)) = self.line.filter(|(a, b)| a != b) else { return guides.collect() };
        let mut out = vec![Overlay::Line { a, b, color: CYAN, dashed: false }];
        // Measurement Labels decides the readout; the line the tool draws stays.
        if cx.measurement_labels {
            let (dist, ang, dx, dy) = measure(a, b);
            out.push(Overlay::Measure {
                p: b + Vec2::new(cx.tol(12.0), cx.tol(12.0)),
                text: format!("D: {}\nAngle: {ang:.1}°\n{}", cx.len(dist), cx.offset_label(dx, dy)),
            });
        }
        out.extend(guides);
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
    use crate::{GlyphSnap, ScreenFrame};
    use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, Selection, TextObject};
    use vectorcraft_geom::kurbo::{ParamCurve, Shape};
    use vectorcraft_geom::{Affine, Rect};
    use vectorcraft_text::{FontDb, layout as layout_text};

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

    /// With Snap to Point alone, an anchor behind the start can pull the projected end behind it;
    /// the end is turned back onto the pointer's step instead of reading 180°.
    #[test]
    fn shift_drag_keeps_its_step_when_an_anchor_lies_behind_the_start() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        cx.snap_tolerance = 4.0;
        cx.snapping_tolerance = 4.0;
        assert!(cx.snap_to_point);
        let shift = Mods { shift: true, ..Mods::default() };
        let mut t = MeasureTool::default();
        // From (200.3, 196.005), the pointer at (202.3, 196.805) points 0° and snaps to the anchor
        // at (200, 200), which projects onto the 0° line at (200, 196.005), behind the start.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.3, 196.005));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 202.3, 196.805).with_mods(shift));
        let o = t.options();
        assert!(o["angle"].as_f64().unwrap().abs() < 1e-9, "{o}");
        assert!((o["dx"].as_f64().unwrap() - 0.3).abs() < 1e-9, "{o}");
        assert!(o["dy"].as_f64().unwrap().abs() < 1e-9, "{o}");
    }

    /// With Snap to Point alone, a Shift-drag's end slides along its 45° step to a ruler guide.
    #[test]
    fn shift_drag_meets_a_ruler_guide() {
        let (mut d, _) = doc_with_rect();
        d.add_ruler_guide(&vectorcraft_doc::Guide::new(true, 50.0));
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

    /// A document holding point type `text` at (100, 300), 60 pt, turned `angle` radians about
    /// where it starts, and the type.
    fn type_doc(text: &str, angle: f64, visible: bool, locked: bool) -> (Document, TextObject) {
        let mut d = Document::new(500.0, 500.0);
        let (l, id) = (d.layers[0].id, d.alloc_id());
        let at = Point::new(100.0, 300.0);
        let mut t = TextObject::point(at, text, CharStyle { size: 60.0, ..Default::default() });
        t.xf = Affine::rotate_about(angle, at) * t.xf;
        let mut n = Node::new(id, NodeKind::Text(Box::new(t.clone())));
        (n.visible, n.locked) = (visible, locked);
        d.insert(Some(l), 0, n).unwrap();
        (d, t)
    }

    /// Where the baseline of `t`'s first line starts on the page, and its unit direction.
    fn baseline(t: &TextObject) -> (Point, Vec2) {
        let lay = layout_text(FontDb::global(), t);
        let (m, line) = (t.xf * lay.line_xf, &lay.lines[0]);
        let (at, along) = (m * Point::new(line.x0, line.baseline), m * Point::new(line.x0 + 1.0, line.baseline));
        (at, (along - at) / (along - at).hypot())
    }

    /// The y of the x-height of `t`'s first glyph, in type that isn't turned.
    fn x_height(t: &TextObject) -> f64 {
        let lay = layout_text(FontDb::global(), t);
        let g = &lay.glyphs[0];
        (t.xf * g.xf * Point::new(0.0, -FontDb::global().face_by_id(g.font_id).unwrap().x_height())).y
    }

    /// The bounds on the page of glyph `i` of `t`, which isn't turned.
    fn glyph_bounds(t: &TextObject, i: usize) -> Rect {
        t.xf.transform_rect_bbox(layout_text(FontDb::global(), t).glyphs[i].outline.bounding_box())
    }

    /// Where a measurement dragged from `from` to `to` starts and ends.
    fn measured(tool: &mut dyn Tool, cx: &ToolContext, from: Point, to: Point, shift: bool) -> (Point, Point) {
        let mods = Mods { shift, ..Mods::default() };
        tool.pointer(cx, &PointerEvent::new(PointerKind::Down, from.x, from.y));
        tool.pointer(cx, &PointerEvent::new(PointerKind::Drag, to.x, to.y).with_mods(mods));
        tool.pointer(cx, &PointerEvent::new(PointerKind::Up, to.x, to.y).with_mods(mods));
        let o = tool.options();
        let a = Point::new(o["x"].as_f64().unwrap(), o["y"].as_f64().unwrap());
        (a, a + Vec2::new(o["dx"].as_f64().unwrap(), o["dy"].as_f64().unwrap()))
    }

    /// Snap to Glyph: both ends land on a line of type's baseline and x-height and on its glyphs'
    /// bounds, each only while its option is on. With Snap to Glyph off they stay at the pointer.
    #[test]
    fn its_ends_land_on_glyph_guides_with_snap_to_glyph() {
        let (d, t) = type_doc("Hg", 0.0, true, false);
        let (s, p) = (Selection::default(), paint());
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let (base, x) = (baseline(&t).0.y, x_height(&t));
        let (from, to) = (Point::new(400.0, base + 2.0), Point::new(450.0, x + 2.0));
        let (a, b) = measured(&mut MeasureTool::default(), &on, from, to, false);
        assert_eq!(a, Point::new(400.0, base), "the press lands on the baseline");
        assert_eq!(b, Point::new(450.0, x), "the end lands on the x-height");
        let off = ToolContext { snap_to_glyph: false, ..on };
        assert_eq!(measured(&mut MeasureTool::default(), &off, from, to, false), (from, to));
        // The Character panel's options: without Baseline and Glyph Bounds (the bottom of `H` is
        // on the baseline) the press stays, and without x-height the end does.
        let o = GlyphSnap { baseline: false, bounds: false, x_height: false, ..GlyphSnap::default() };
        let without = ToolContext { glyph_snap: o, ..on };
        assert_eq!(measured(&mut MeasureTool::default(), &without, from, to, false), (from, to));
        // A corner of the box of `H`, which only Glyph Bounds gives there.
        let corner = Point::new(glyph_bounds(&t, 0).x1, glyph_bounds(&t, 0).y0);
        let near = corner + Vec2::new(1.5, -1.0);
        let bounds = ToolContext { glyph_snap: GlyphSnap { anchors: false, ..GlyphSnap::default() }, ..on };
        let landed = measured(&mut MeasureTool::default(), &bounds, Point::new(400.0, 50.0), near, false).1;
        assert!(landed.distance(corner) < 1e-9, "{landed:?} vs {corner:?}");
        let plain = measured(&mut MeasureTool::default(), &off, Point::new(400.0, 50.0), near, false).1;
        assert!((plain.x - corner.x).abs() > 1e-3, "{plain:?}");
    }

    /// A Shift-drag's end stays on its 45° step and slides along it to where it meets a glyph guide.
    #[test]
    fn a_shift_drag_slides_along_its_step_to_a_glyph_guide() {
        let (d, t) = type_doc("Hg", 0.0, true, false);
        let (s, p) = (Selection::default(), paint());
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let x = x_height(&t);
        let (from, to) = (Point::new(300.0, x - 100.0), Point::new(401.0, x + 1.0));
        let landed = measured(&mut MeasureTool::default(), &on, from, to, true).1;
        assert!(landed.distance(Point::new(400.0, x)) < 1e-9, "{landed:?}");
        let off = ToolContext { snap_to_glyph: false, ..on };
        let stayed = measured(&mut MeasureTool::default(), &off, from, to, true).1;
        assert!(stayed.distance(to) < 1e-9, "{stayed:?}");
    }

    /// Proximity Guides pull an end, with every other option off. Anchor Point does not.
    #[test]
    fn proximity_guides_pull_an_end_and_a_glyphs_anchor_points_do_not() {
        let (d, t) = type_doc("HHHH", 0.0, true, false);
        let (s, p) = (Selection::default(), paint());
        let none = GlyphSnap { baseline: false, x_height: false, bounds: false, proximity: false, angular: false, anchors: false };
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let off = ToolContext { snap_to_glyph: false, ..on };
        let start = Point::new(400.0, 50.0);
        let end = |c: &ToolContext, to: Point| measured(&mut MeasureTool::default(), c, start, to, false).1;
        let lay = layout_text(FontDb::global(), &t);
        let edge = (t.xf * lay.glyphs[1].origin).x;
        let to = Point::new(edge + 2.0, 100.0);
        assert_eq!(end(&ToolContext { glyph_snap: GlyphSnap { proximity: true, ..none }, ..on }, to).x, edge);
        assert_eq!(end(&off, to).x, to.x);
        let anchor = t.xf * lay.glyphs[1].outline.segments().next().unwrap().start();
        let to = anchor + Vec2::new(1.5, -1.0);
        assert_eq!(end(&ToolContext { glyph_snap: GlyphSnap { anchors: true, ..none }, ..on }, to), end(&off, to));
    }

    /// While a drag snaps with Snap to Glyph on, the guide shows; the drag's end hides it, and with
    /// Snap to Glyph off there is only the measured line and its readout.
    #[test]
    fn a_drag_shows_the_glyph_guide_it_lands_on() {
        let (d, t) = type_doc("Hg", 0.0, true, false);
        let (s, p) = (Selection::default(), paint());
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let to = Point::new(450.0, baseline(&t).0.y + 2.0);
        let guides = |c: &ToolContext, to: Point, up: bool| {
            let mut tool = MeasureTool::default();
            tool.pointer(c, &PointerEvent::new(PointerKind::Down, 400.0, 50.0));
            tool.pointer(c, &PointerEvent::new(if up { PointerKind::Up } else { PointerKind::Drag }, to.x, to.y));
            tool.overlays(c).iter().filter(|o| !matches!(o, Overlay::Measure { .. })).count() - 1
        };
        assert!(guides(&on, to, false) > 0, "the guide shows during the drag");
        assert_eq!(guides(&on, to, true), 0, "the guide goes when the drag ends");
        // Off, an end still lines up with a rectangle's top edge, as it always has, with no guide.
        let (rect, _) = doc_with_rect();
        let off = ToolContext { snap_to_glyph: false, ..cx(&rect, &s, &p) };
        let near = Point::new(450.0, 101.0);
        assert_eq!(measured(&mut MeasureTool::default(), &off, Point::new(400.0, 50.0), near, false).1.y, 100.0);
        assert_eq!(guides(&off, near, false), 0, "no guide with Snap to Glyph off");
    }

    /// In rotated type the guides run at its angle: the end lands on the baseline, only with
    /// Angular Guides on.
    #[test]
    fn its_ends_land_on_the_guides_of_rotated_type() {
        let (d, t) = type_doc("H", 30f64.to_radians(), true, false);
        let (s, p) = (Selection::default(), paint());
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let (at, dir) = baseline(&t);
        let to = at + dir * 250.0 + Vec2::new(-dir.y, dir.x) * 2.0;
        let off_line = |c: &ToolContext| (measured(&mut MeasureTool::default(), c, Point::new(20.0, 20.0), to, false).1 - at).cross(dir).abs();
        assert!(off_line(&on) < 1e-6, "{}", off_line(&on));
        let flat = ToolContext { glyph_snap: GlyphSnap { angular: false, ..GlyphSnap::default() }, ..on };
        assert!(off_line(&flat) > 1.0, "{}", off_line(&flat));
    }

    /// A paragraph gives glyph bounds only: no x-height, and the box of a glyph on a later line.
    #[test]
    fn a_paragraph_gives_the_measure_tool_glyph_bounds_only() {
        let (d, t) = type_doc("H\nH", 0.0, true, false);
        let (s, p) = (Selection::default(), paint());
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let start = Point::new(400.0, 50.0);
        let to = Point::new(450.0, x_height(&t) + 2.0);
        assert_eq!(measured(&mut MeasureTool::default(), &on, start, to, false).1, to, "no x-height guide");
        let lay = layout_text(FontDb::global(), &t);
        assert_eq!(lay.lines.len(), 2);
        let top = glyph_bounds(&t, lay.glyphs.len() - 1).y0;
        assert_eq!(measured(&mut MeasureTool::default(), &on, start, Point::new(450.0, top - 2.0), false).1, Point::new(450.0, top));
    }

    /// Hidden type gives no guides. Locked type does, as it does for the drawing tools.
    #[test]
    fn hidden_type_gives_no_glyph_guides_and_locked_type_does() {
        let (s, p) = (Selection::default(), paint());
        let drag = |d: &Document, base: f64| {
            let c = ToolContext { snap_to_glyph: true, ..cx(d, &s, &p) };
            measured(&mut MeasureTool::default(), &c, Point::new(400.0, 50.0), Point::new(450.0, base + 2.0), false).1
        };
        let (hidden, t) = type_doc("Hg", 0.0, false, false);
        let base = baseline(&t).0.y;
        assert_eq!(drag(&hidden, base).y, base + 2.0);
        let (locked, _) = type_doc("Hg", 0.0, true, true);
        assert_eq!(drag(&locked, base).y, base);
    }

    /// Grid and pixel snapping come first; Snap to Glyph needs Smart Guides and Alignment Guides.
    #[test]
    fn glyph_guides_keep_their_place_among_the_other_snaps() {
        let (d, t) = type_doc("Hg", 0.0, true, false);
        let (s, p) = (Selection::default(), paint());
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let base = baseline(&t).0.y;
        let (start, to) = (Point::new(400.0, 50.0), Point::new(450.0, base + 2.0));
        let end = |c: &ToolContext| measured(&mut MeasureTool::default(), c, start, to, false).1;
        assert_eq!(end(&on), Point::new(to.x, base));
        let grid = ToolContext { snap_to_grid: true, ..on };
        assert_eq!(end(&grid), vectorcraft_geom::snap::snap_point_to_grid(to, grid.grid_step()));
        assert_eq!(end(&ToolContext { snap_to_pixel: true, ..on }), Point::new(to.x.round(), to.y.round()));
        assert_eq!(end(&ToolContext { smart_guides: false, snap_to_point: true, ..on }), to);
        assert_eq!(end(&ToolContext { alignment_guides: false, ..on }), to);
    }

    /// The snapping distance is in screen pixels, so a zoomed-out view reaches farther; type
    /// outside the window gives no guides.
    #[test]
    fn glyph_guides_follow_the_view() {
        let (d, t) = type_doc("Hg", 0.0, true, false);
        let (s, p) = (Selection::default(), paint());
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let base = baseline(&t).0.y;
        let (start, to) = (Point::new(400.0, 50.0), Point::new(450.0, base - 5.0));
        let end = |c: &ToolContext| measured(&mut MeasureTool::default(), c, start, to, false).1;
        assert_eq!(end(&on), to, "5 pt is out of reach at 100%");
        assert_eq!(end(&ToolContext { zoom: 0.25, ..on }), Point::new(to.x, base));
        let frame = |origin: Point| ScreenFrame { origin, right: Vec2::new(1.0, 0.0), down: Vec2::new(0.0, 1.0), size: (800.0, 600.0) };
        let near = Point::new(to.x, base + 2.0);
        let end = |c: &ToolContext| measured(&mut MeasureTool::default(), c, start, near, false).1;
        assert_eq!(end(&ToolContext { screen: Some(frame(Point::ZERO)), ..on }), Point::new(to.x, base));
        assert_eq!(end(&ToolContext { screen: Some(frame(Point::new(0.0, 1000.0))), ..on }), near);
    }

    /// A drag lays out the type once: it keeps the guides its press gathered until the document
    /// or a snapping option changes.
    #[test]
    fn a_drag_keeps_the_glyph_guides_its_press_gathered() {
        let (d, t) = type_doc("Hg", 0.0, true, false);
        let (empty, s, p) = (Document::new(500.0, 500.0), Selection::default(), paint());
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let base = baseline(&t).0.y;
        let to = Point::new(450.0, base + 2.0);
        let end = |tool: &MeasureTool| {
            let o = tool.options();
            o["y"].as_f64().unwrap() + o["dy"].as_f64().unwrap()
        };
        let mut tool = MeasureTool::default();
        tool.pointer(&on, &PointerEvent::new(PointerKind::Down, 400.0, 50.0));
        // The same revision: the guides of the press still serve, though this document has no type.
        let later = ToolContext { snap_to_glyph: true, ..cx(&empty, &s, &p) };
        tool.pointer(&later, &PointerEvent::new(PointerKind::Drag, to.x, to.y));
        assert_eq!(end(&tool), base);
        // A new revision gathers them again.
        let edited = ToolContext { revision: (1, 0), ..later };
        tool.pointer(&edited, &PointerEvent::new(PointerKind::Drag, to.x, to.y));
        assert_eq!(end(&tool), to.y);
        // So does turning Snap to Glyph off during the drag.
        let mut tool = MeasureTool::default();
        tool.pointer(&on, &PointerEvent::new(PointerKind::Down, 400.0, 50.0));
        tool.pointer(&ToolContext { snap_to_glyph: false, ..on }, &PointerEvent::new(PointerKind::Drag, to.x, to.y));
        assert_eq!(end(&tool), to.y);
    }

    /// Escape brings back the last result, and the next drag still lands on the guides.
    #[test]
    fn escape_leaves_the_glyph_guides_for_the_next_drag() {
        let (d, t) = type_doc("Hg", 0.0, true, false);
        let (s, p) = (Selection::default(), paint());
        let on = ToolContext { snap_to_glyph: true, ..cx(&d, &s, &p) };
        let base = baseline(&t).0.y;
        let mut tool = MeasureTool::default();
        let to = Point::new(450.0, base + 2.0);
        let first = measured(&mut tool, &on, Point::new(400.0, 50.0), to, false);
        tool.pointer(&on, &PointerEvent::new(PointerKind::Down, 300.0, 60.0));
        tool.pointer(&on, &PointerEvent::new(PointerKind::Drag, 320.0, base + 2.0));
        tool.key(&on, ToolKey::Escape, Mods::default());
        assert!((tool.options()["y"].as_f64().unwrap() - first.0.y).abs() < 1e-9);
        assert_eq!(measured(&mut tool, &on, Point::new(300.0, 60.0), to, false).1, Point::new(450.0, base));
    }
}
