//! Trim at Crossings Tool (Path Precision Tools): a click removes the stretch of the path under the
//! pointer between the places where other paths, or the path itself, cross it; a drag draws a
//! rectangle and removes every stretch it touches; an Alt-drag draws a freehand line and removes
//! every stretch it crosses. Each runs `addon.pathtools.trim` on release, as one undo step, on
//! every path that can be edited, selected or not. Escape during a drag removes nothing.

use serde_json::json;
use vectorcraft_geom::{Point, Rect};

use crate::draw2::FEEDBACK;
use crate::xform::polygon;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "trimAtCrossings";
/// How near the pointer a path counts as under it (screen pixels).
pub const REACH: f64 = 4.0;
/// A press that moves less than this many screen pixels is a click.
const MIN_DRAG_PX: f64 = 3.0;
/// The most points a freehand line keeps (one per screen pixel dragged). A longer one is dropped.
pub(crate) const MAX_POINTS: usize = 65_536;
/// The command the tool runs.
const COMMAND: &str = "addon.pathtools.trim";

#[derive(Default)]
pub struct TrimTool {
    /// Where the press started, while the button is down.
    start: Option<Point>,
    end: Point,
    /// An Alt-drag's freehand line (empty for a rectangle).
    line: Vec<Point>,
    freehand: bool,
}

impl TrimTool {
    fn reset(&mut self) {
        self.start = None;
        self.line.clear();
        self.freehand = false;
    }
}

impl Tool for TrimTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.start.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if !ev.pos.is_finite() {
            if ev.kind == PointerKind::Up {
                self.reset();
            }
            return vec![];
        }
        match ev.kind {
            PointerKind::Down => {
                self.start = Some(ev.pos);
                self.end = ev.pos;
                self.freehand = ev.mods.alt;
                self.line = if self.freehand { vec![ev.pos] } else { vec![] };
                vec![]
            }
            PointerKind::Drag if self.start.is_some() => {
                self.end = ev.pos;
                if self.freehand && self.line.last().is_none_or(|p| p.distance(ev.pos) >= cx.tol(1.0)) {
                    // A line cut short would cross other stretches than the one drawn: drop it.
                    if self.line.len() >= MAX_POINTS {
                        self.reset();
                        return vec![];
                    }
                    self.line.push(ev.pos);
                }
                vec![]
            }
            PointerKind::Up => {
                let Some(start) = self.start else { return vec![] };
                let mut line = std::mem::take(&mut self.line);
                let freehand = self.freehand;
                self.reset();
                // A press that stayed near where it started is a click, even when a freehand line
                // took a few points on the way.
                let near = |p: &Point| start.distance(*p) < cx.tol(MIN_DRAG_PX);
                if near(&ev.pos) && line.iter().all(near) {
                    return vec![Action::Exec(COMMAND.into(), json!({"at": [start.x, start.y], "tolerance": cx.tol(REACH).min(100.0)}))];
                }
                if !freehand {
                    let r = Rect::from_points(start, ev.pos);
                    return vec![Action::Exec(COMMAND.into(), json!({"rect": [r.x0, r.y0, r.width(), r.height()]}))];
                }
                if line.last().is_some_and(|p| *p != ev.pos) {
                    line.push(ev.pos);
                }
                let points: Vec<_> = line.iter().map(|p| json!([p.x, p.y])).collect();
                if points.len() < 2 { vec![] } else { vec![Action::Exec(COMMAND.into(), json!({ "points": points }))] }
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.reset();
        }
        vec![]
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        self.start.is_some() && key == ToolKey::Escape
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(start) = self.start else { return vec![] };
        if self.freehand {
            return if self.line.len() < 2 {
                vec![]
            } else {
                vec![Overlay::Path { path: polygon(&self.line, false), color: FEEDBACK, width: 1.0, dashed: false }]
            };
        }
        if start.distance(self.end) < cx.tol(MIN_DRAG_PX) { vec![] } else { vec![Overlay::Marquee(Rect::from_points(start, self.end))] }
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn a_click_trims_at_the_point_with_nothing_selected() {
        let (d, _) = doc_with_rect();
        let (sel, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &sel, &p);
        cx.zoom = 2.0;
        let mut t = TrimTool::default();
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0)).is_empty());
        assert!(t.busy() && t.overlays(&cx).is_empty());
        let up = t.pointer(&cx, &ev(PointerKind::Up, 100.5, 100.0));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"at": [100.0, 100.0], "tolerance": 2.0}))], "4 px at 200%");
        assert!(!t.busy());
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::Crosshair);
    }

    #[test]
    fn a_drag_trims_what_its_rectangle_touches_and_escape_drops_it() {
        let (d, _) = doc_with_rect();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 160.0, 130.0));
        assert!(t.pointer(&cx, &ev(PointerKind::Drag, 140.0, 90.0)).is_empty(), "nothing runs while dragging");
        assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Marquee(r)] if *r == Rect::new(140.0, 90.0, 160.0, 130.0)));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 140.0, 90.0));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"rect": [140.0, 90.0, 20.0, 40.0]}))]);
        // Escape during a drag: nothing runs on release, and the key is the tool's only then.
        assert!(!t.claims_key(&cx, ToolKey::Escape));
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 60.0, 60.0));
        assert!(t.claims_key(&cx, ToolKey::Escape));
        assert!(t.key(&cx, ToolKey::Escape, Mods::default()).is_empty());
        assert!(!t.busy() && t.overlays(&cx).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 60.0, 60.0)).is_empty());
    }

    #[test]
    fn an_alt_drag_trims_what_its_line_crosses() {
        let (d, _) = doc_with_rect();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let alt = Mods { alt: true, ..Mods::default() };
        let mut t = TrimTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 10.2, 10.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 40.0, 30.0));
        assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Path { .. }]));
        // Alt at the press decides; points less than a pixel apart are one.
        let up = t.pointer(&cx, &ev(PointerKind::Up, 80.0, 30.0));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"points": [[10.0, 10.0], [40.0, 30.0], [80.0, 30.0]]}))]);
        // An Alt-click is a click.
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0).with_mods(alt));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 10.0, 10.0).with_mods(alt));
        assert!(matches!(up.as_slice(), [Action::Exec(_, v)] if v.get("at").is_some()), "{up:?}");
        // So is one that moves a pixel or two, and one that goes away and comes back is a line.
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 12.0, 10.0).with_mods(alt));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 12.0, 11.0).with_mods(alt));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"at": [10.0, 10.0], "tolerance": 4.0}))]);
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 40.0, 10.0).with_mods(alt));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 10.0, 11.0).with_mods(alt));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"points": [[10.0, 10.0], [40.0, 10.0], [10.0, 11.0]]}))]);
        // A line past the cap is dropped rather than cut short.
        t.pointer(&cx, &ev(PointerKind::Down, 0.0, 0.0).with_mods(alt));
        for i in 1..=MAX_POINTS {
            t.pointer(&cx, &ev(PointerKind::Drag, i as f64 * 2.0, 0.0));
        }
        assert!(!t.busy());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 0.0, 0.0)).is_empty());
    }
}
