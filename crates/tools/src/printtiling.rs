//! The Print Tiling tool: drag to put the printed pages where they print. The pointer is the
//! top-left corner of the first page's imageable area (snapping to the artboard's edges); the
//! drag previews `print.tiling.set` and commits it as one undo step. A double click puts the pages
//! back where the placement puts them (`print.tiling.set {reset}`). The canvas shows the pages
//! while the tool is active (as View → Show Print Tiling does).

use serde_json::json;
use vectorcraft_geom::{Point, Rect};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// Create the Print Tiling tool by id (None = not ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    (id == "printTiling").then(|| Box::new(PrintTilingTool::default()) as Box<dyn Tool>)
}

#[derive(Default)]
pub struct PrintTilingTool {
    /// The artboard the drag started on (index and rect), while dragging.
    drag: Option<(Option<usize>, Option<Rect>)>,
    began: bool,
    last: Point,
}

/// `v` snapped to the nearest of `edges` within `tol`.
fn snap1(v: f64, edges: [f64; 2], tol: f64) -> f64 {
    edges.into_iter().filter(|e| (e - v).abs() <= tol).min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs())).unwrap_or(v)
}

impl Tool for PrintTilingTool {
    fn id(&self) -> &'static str {
        "printTiling"
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                let hit = cx.doc.artboard_holding(ev.pos, false);
                self.drag = Some((hit, hit.and_then(|i| cx.doc.artboards.get(i)).map(|a| a.rect)));
                (self.began, self.last) = (false, ev.pos);
                vec![]
            }
            PointerKind::Drag => {
                let Some((artboard, rect)) = self.drag else { return vec![] };
                let tol = cx.tol(5.0);
                let p = match rect {
                    Some(r) => Point::new(snap1(ev.pos.x, [r.x0, r.x1], tol), snap1(ev.pos.y, [r.y0, r.y1], tol)),
                    None => ev.pos,
                };
                self.last = p;
                let mut out = vec![];
                if !self.began {
                    self.began = true;
                    out.push(Action::Begin("Print Tiling".into()));
                }
                out.push(Action::Preview("print.tiling.set".into(), json!({ "origin": [p.x, p.y], "artboard": artboard })));
                out
            }
            PointerKind::Up => {
                let began = std::mem::take(&mut self.began);
                if self.drag.take().is_some() && began { vec![Action::Commit] } else { vec![] }
            }
            PointerKind::DoubleClick => {
                self.drag = None;
                vec![Action::Exec("print.tiling.set".into(), json!({ "reset": true }))]
            }
            PointerKind::Move => vec![],
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape { self.deactivate(cx) } else { vec![] }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        match self.drag {
            // How far the corner is from the artboard's.
            Some((_, Some(r))) if self.began && cx.measurement_labels => {
                vec![Overlay::Measure { p: self.last, text: cx.offset_label(self.last.x - r.x0, self.last.y - r.y0) }]
            }
            _ => vec![],
        }
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::PrintTiling
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.drag = None;
        if std::mem::take(&mut self.began) { vec![Action::Cancel] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_doc::Selection;

    use super::*;
    use crate::testutil::{cx, doc_with_rect, paint};

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn a_drag_previews_the_origin_and_commits_once() {
        let (d, _) = doc_with_rect();
        let (s, pd) = (Selection::default(), paint());
        let c = cx(&d, &s, &pd);
        let mut t = PrintTilingTool::default();
        assert!(t.pointer(&c, &ev(PointerKind::Down, 50.0, 60.0)).is_empty());
        let a = t.pointer(&c, &ev(PointerKind::Drag, 40.0, 30.0));
        assert_eq!(a.first(), Some(&Action::Begin("Print Tiling".into())));
        assert_eq!(a.get(1), Some(&Action::Preview("print.tiling.set".into(), json!({"origin": [40.0, 30.0], "artboard": 0}))));
        // Near the artboard's edge the corner snaps to it.
        let a = t.pointer(&c, &ev(PointerKind::Drag, 2.0, 497.0));
        assert_eq!(a, vec![Action::Preview("print.tiling.set".into(), json!({"origin": [0.0, 500.0], "artboard": 0}))]);
        assert!(matches!(t.overlays(&c).as_slice(), [Overlay::Measure { .. }]));
        let quiet = ToolContext { measurement_labels: false, ..cx(&d, &s, &pd) };
        assert!(t.overlays(&quiet).is_empty());
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 2.0, 497.0)), vec![Action::Commit]);
        assert!(!t.busy());
        // A click alone changes nothing; a double click resets.
        t.pointer(&c, &ev(PointerKind::Down, 10.0, 10.0));
        assert!(t.pointer(&c, &ev(PointerKind::Up, 10.0, 10.0)).is_empty());
        assert_eq!(t.pointer(&c, &ev(PointerKind::DoubleClick, 10.0, 10.0)), vec![Action::Exec("print.tiling.set".into(), json!({"reset": true}))]);
        // Escape mid-drag cancels.
        t.pointer(&c, &ev(PointerKind::Down, 10.0, 10.0));
        t.pointer(&c, &ev(PointerKind::Drag, 20.0, 20.0));
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert_eq!(crate::create("printTiling").id(), "printTiling");
    }

    #[test]
    fn a_press_on_an_artboards_right_or_bottom_edge_drags_from_that_artboard() {
        // Artboard 0 is (0, 0) to (500, 500). Artboard 1 is (500, 0) to (800, 200), and its left edge lies on artboard 0's right edge.
        let (mut d, _) = doc_with_rect();
        let mut ab = d.artboards[0].clone();
        ab.rect = Rect::new(500.0, 0.0, 800.0, 200.0);
        d.artboards.push(ab);
        let (s, pd) = (Selection::default(), paint());
        let c = cx(&d, &s, &pd);
        // What a drag to 3 pt up and left of a press at (x, y) previews: the corner snaps to the
        // edges of the artboard the press picked.
        let drag = |x: f64, y: f64| {
            let mut t = PrintTilingTool::default();
            t.pointer(&c, &ev(PointerKind::Down, x, y));
            t.pointer(&c, &ev(PointerKind::Drag, x - 3.0, y - 3.0)).pop()
        };
        let preview =
            |origin: [f64; 2], artboard: Value| Some(Action::Preview("print.tiling.set".into(), json!({ "origin": origin, "artboard": artboard })));
        assert_eq!(drag(800.0, 100.0), preview([800.0, 97.0], json!(1)), "artboard 1's right edge");
        assert_eq!(drag(650.0, 200.0), preview([647.0, 200.0], json!(1)), "artboard 1's bottom edge");
        assert_eq!(drag(800.0, 200.0), preview([800.0, 200.0], json!(1)), "artboard 1's bottom right corner");
        assert_eq!(drag(300.0, 500.0), preview([297.0, 500.0], json!(0)), "artboard 0's bottom edge");
        assert_eq!(drag(500.0, 100.0), preview([500.0, 97.0], json!(1)), "the shared edge: the artboard to its right");
        assert_eq!(drag(800.0 + 1e-9, 100.0), preview([800.0, 97.0], json!(1)), "roundoff past the edge");
        assert_eq!(drag(800.001, 100.0), preview([800.001 - 3.0, 97.0], Value::Null), "beyond the edge");
    }
}
