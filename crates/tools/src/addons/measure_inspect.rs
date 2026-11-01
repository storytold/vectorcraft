//! Measure & Inspect tool (Path Precision Tools): drag between two points to read their distance,
//! angle and offsets (Shift: 45° steps), which stay shown until the next drag; hover a path to
//! read it there: open or closed and its anchor count, the segment's and the path's length, the
//! area it encloses, the radius of curvature and the direction it runs at the pointer, and over an
//! anchor its angle and handles. Readouts are in the document's units; nothing is drawn into the
//! document (`addon.pathtools.inspect` gives agents the same readouts).

use serde_json::{Value, json};
use vectorcraft_doc::hit::{HitOptions, hit_test};
use vectorcraft_doc::{NodeId, Unit};
use vectorcraft_geom::{Point, Vec2};
use vectorcraft_pathops::addons::pathtools::{Inspection, inspect};

use crate::xform::measure;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "measureInspect";
const CYAN: [u8; 3] = [0x00, 0xa8, 0xff];
const ORANGE: [u8; 3] = [0xff, 0x8a, 0x00];

#[derive(Default)]
pub struct MeasureInspectTool {
    line: Option<(Point, Point)>,
    dragging: bool,
    /// The line measured before the drag under way, which Escape puts back.
    before: Option<(Point, Point)>,
    /// The path under the pointer and what it reads there.
    hover: Option<(NodeId, Inspection)>,
}

/// An area of `pt2` square points in `unit` (`12.5 mm²`).
fn area_label(unit: Unit, pt2: f64) -> String {
    let side = unit.from_pt(1.0);
    format!("{:.2} {}²", pt2 * side * side, unit.suffix())
}

/// The readouts of `i`, a line each.
fn readout(cx: &ToolContext, i: &Inspection) -> String {
    let mut lines = vec![
        format!("{} · {} anchors", if i.closed { "Closed path" } else { "Open path" }, i.anchors),
        format!("Segment: {}", cx.len(i.segment_length)),
        format!("Path: {}", cx.len(i.length)),
        format!("Area: {}", area_label(cx.unit, i.area)),
        format!("Tangent: {:.1}°", i.tangent),
        match i.radius {
            Some(r) => format!("Radius: {}", cx.len(r)),
            None => "Radius: straight".into(),
        },
    ];
    if let Some(a) = &i.anchor {
        let kind = if a.smooth { "smooth" } else { "corner" };
        lines.push(match a.angle {
            Some(deg) => format!("Anchor {} ({kind}) · Angle: {deg:.1}°", a.index + 1),
            None => format!("Anchor {} ({kind}) · open end", a.index + 1),
        });
        for (name, h) in [("In", a.handle_in), ("Out", a.handle_out)] {
            if let Some((len, deg)) = h {
                lines.push(format!("{name}: {} at {deg:.1}°", cx.len(len)));
            }
        }
    }
    lines.join("\n")
}

impl MeasureInspectTool {
    /// The path whose outline is under `p`, and its readouts there.
    fn inspect_at(cx: &ToolContext, p: Point) -> Option<(NodeId, Inspection)> {
        // Outlines only: a fill under the pointer is not a path to read.
        let hit = hit_test(cx.doc, p, HitOptions { outline: true, ..cx.hit_options() })?;
        let path = cx.doc.node(hit.leaf)?.path_data()?;
        let i = inspect(path, p, cx.point_tol())?;
        (i.point.distance(p) <= cx.pick_tol() * 2.0).then_some((hit.leaf, i))
    }
}

impl Tool for MeasureInspectTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.dragging
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => self.hover = Self::inspect_at(cx, ev.pos),
            PointerKind::Down => {
                let (p, _) = crate::guides::snap_draw(cx, ev.pos, &[]);
                self.before = self.line;
                self.line = Some((p, p));
                self.dragging = true;
                self.hover = None;
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

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        let off = Vec2::new(cx.tol(14.0), cx.tol(14.0));
        if let Some((a, b)) = self.line.filter(|(a, b)| a != b) {
            let (dist, ang, dx, dy) = measure(a, b);
            out.push(Overlay::Line { a, b, color: CYAN, dashed: false });
            out.push(Overlay::Measure { p: b + off, text: format!("D: {}\nAngle: {ang:.1}°\n{}", cx.len(dist), cx.offset_label(dx, dy)) });
        }
        if let Some((id, i)) = self.hover.as_ref().filter(|_| !self.dragging) {
            if let Some(sp) =
                cx.doc.node(*id).and_then(|n| n.path_data()).and_then(|p| p.subpaths.get(i.subpath)).filter(|sp| i.segment < sp.segment_count())
            {
                let c = sp.segment(i.segment);
                let mut bp = vectorcraft_geom::BezPath::new();
                bp.move_to(c.p0);
                bp.curve_to(c.p1, c.p2, c.p3);
                out.push(Overlay::Path { path: bp, color: ORANGE, width: 2.0, dashed: false });
            }
            let at = i.anchor.as_ref().map_or(i.point, |a| a.p);
            out.push(Overlay::Anchor { p: at, color: ORANGE, filled: i.anchor.is_some(), size: 6.0 });
            out.push(Overlay::Measure { p: at + off, text: readout(cx, i) });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::MeasureInspect
    }

    /// The last measurement, as the Measure tool reports it.
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
    fn a_drag_measures_and_hovering_a_path_reads_it() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = MeasureInspectTool::default();
        for (k, x, y) in [(PointerKind::Down, 10.0, 10.0), (PointerKind::Drag, 40.0, -30.0), (PointerKind::Up, 40.0, -30.0)] {
            assert!(t.pointer(&cx, &PointerEvent::new(k, x, y)).is_empty(), "nothing goes into the document");
        }
        assert!((t.options()["distance"].as_f64().unwrap() - 50.0).abs() < 1e-9);
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.starts_with("D: 50"))));
        // Hover the rectangle's top edge (100, 100)–(200, 100), then its corner.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 150.0, 101.0));
        let text = t
            .overlays(&cx)
            .into_iter()
            .filter_map(|o| if let Overlay::Measure { text, .. } = o { Some(text) } else { None })
            .find(|t| t.contains("Segment"))
            .expect("the readout");
        assert!(text.contains("Closed path · 4 anchors") && text.contains("Segment: 100") && text.contains("Path: 400"), "{text}");
        assert!(text.contains("Area: 10000.00 pt²") && text.contains("Radius: straight"), "{text}");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 199.0, 101.0));
        let ov = t.overlays(&cx);
        assert!(ov.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.contains("Angle: 90.0°"))), "{ov:?}");
        // Away from every outline (inside the fill) there is nothing to read.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 150.0, 150.0));
        assert!(!t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.contains("Segment"))));
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::MeasureInspect);
    }

    #[test]
    fn escape_puts_back_the_measurement_before() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = MeasureInspectTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 30.0, 0.0));
        t.key(&cx, ToolKey::Escape, Mods::default());
        assert!(!t.busy());
        assert_eq!(t.options(), Value::Null);
        assert_eq!(area_label(Unit::Points, 2.0), "2.00 pt²");
    }
}
