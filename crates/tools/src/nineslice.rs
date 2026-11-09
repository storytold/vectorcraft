//! The 9-slice guides of a symbol edited in symbol editing mode: two horizontal and two vertical
//! guides over its art
//! ([helpx: maintain proportions while scaling symbols](https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/maintain-proportions-while-scaling-symbols.html)).
//! The canvas draws them, dotted, with any tool. With the Selection tool, the pointer near a guide
//! takes the Move cursor, and dragging the guide moves it (`symbol.options`'s
//! `nineSliceGuides`), in one undo step.
//!
//! VectorCraft's own rules: a guide stays within the art's box and stops short of the opposite
//! guide; the drag does not snap and its modifiers change nothing; a guide within reach takes the
//! press before the art and the bounding box, and of two guides within reach the nearer one, else
//! the first of left, top, right and bottom; Escape during the drag puts the guide back.

use serde_json::json;
use vectorcraft_doc::{EditedNineSlice, NineSliceGuide};
use vectorcraft_geom::Point;

use crate::{Action, PointerEvent, ToolContext};

/// How near (screen px) the pointer must come to a guide to take it.
const HIT_PX: f64 = 4.0;

/// The distance from `p` to the segment from `a` to `b`.
fn to_segment(p: Point, a: Point, b: Point) -> f64 {
    let d = b - a;
    let len2 = d.hypot2();
    let t = if len2 > 0.0 { ((p - a).dot(d) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + d * t)
}

/// The edited symbol's guides and the one within reach of `p`, the nearest first.
pub fn guide_at(cx: &ToolContext, p: Point) -> Option<(EditedNineSlice, NineSliceGuide)> {
    if !(p.x.is_finite() && p.y.is_finite()) {
        return None;
    }
    let e = cx.doc.edited_nine_slice()?;
    let reach = cx.tol(HIT_PX);
    let near = e
        .lines()
        .into_iter()
        .map(|(g, a, b)| (g, to_segment(p, a, b)))
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(g, _)| g)?;
    Some((e, near))
}

/// Is `p` over a guide of the edited symbol?
pub fn over_guide(cx: &ToolContext, p: Point) -> bool {
    guide_at(cx, p).is_some()
}

/// Dragging a 9-slice guide.
#[derive(Clone, Debug)]
pub struct GuideDrag {
    frame: EditedNineSlice,
    guide: NineSliceGuide,
    start: Point,
    began: bool,
}

impl GuideDrag {
    /// Start a drag when the press `ev` is on a guide of the edited symbol.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let (frame, guide) = guide_at(cx, ev.pos)?;
        Some(Self { frame, guide, start: ev.pos, began: false })
    }

    /// The drag has left the click: the guide is moving.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    pub fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        if !(p.x.is_finite() && p.y.is_finite()) || (!self.began && p.distance(self.start) < cx.tol(3.0)) {
            return vec![];
        }
        let mut out = vec![];
        if !self.began {
            self.began = true;
            out.push(Action::Begin("Symbol Options".into()));
        }
        let guides = self.frame.moved(self.guide, p);
        out.push(Action::Preview("symbol.options".into(), json!({ "name": self.frame.symbol, "nineSliceGuides": guides.to_value() })));
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    pub fn cancel(self) -> Vec<Action> {
        if self.began { vec![Action::Cancel] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::select::SelectionTool;
    use crate::testutil::*;
    use crate::{Cursor, Mods, PointerKind, Tool, ToolKey};
    use vectorcraft_doc::{Appearance, Document, Node, Selection, Symbol, SymbolEdit, SymbolNineSlice};
    use vectorcraft_geom::{Affine, Rect, shapes};

    /// A 100 × 100 symbol "S" with 9-slice guides `guides`, edited in symbol editing mode with its
    /// art, a filled square, at (100, 100) on the editing layer: its guides lie at x and y = 125
    /// and 175.
    fn editing(guides: Option<SymbolNineSlice>) -> Document {
        let mut d = Document::new(500.0, 500.0);
        let square = |d: &mut Document, r: Rect| Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default_art());
        let art = square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0));
        d.symbols.push(Symbol { dynamic: false, name: "S".into(), art: Arc::new(art) });
        d.set_symbol_nine_slice("S", guides);
        let layer = d.add_layer(Some("S"));
        let copy = square(&mut d, Rect::new(100.0, 100.0, 200.0, 200.0));
        d.insert(Some(layer), 0, copy).unwrap();
        d.symbol_edit = Some(SymbolEdit {
            symbol: "S".into(),
            layer,
            instance: None,
            xf: Affine::translate((100.0, 100.0)),
            group: false,
            changed: false,
            back_isolation: None,
            back_layer: None,
        });
        d
    }

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    fn preview(a: &Action) -> (String, serde_json::Value) {
        match a {
            Action::Preview(cmd, v) => (cmd.clone(), v.clone()),
            other => panic!("{other:?}"),
        }
    }

    /// The pointer within 4 screen px of a guide takes it at any zoom, the nearer of two where
    /// they cross, and nothing is taken outside symbol editing mode or without guides.
    #[test]
    fn a_guide_is_taken_within_a_few_screen_pixels_at_any_zoom() {
        let d = editing(Some(SymbolNineSlice::default()));
        let (s, p) = (Selection::default(), paint());
        for zoom in [0.25, 1.0, 8.0] {
            let c = ToolContext { zoom, ..cx(&d, &s, &p) };
            let at = |x: f64, y: f64| guide_at(&c, Point::new(x, y)).map(|(_, g)| g);
            assert_eq!(at(125.0 + 3.5 / zoom, 150.0), Some(NineSliceGuide::Left), "{zoom}");
            assert_eq!(at(125.0 + 5.0 / zoom, 150.0), None, "{zoom}");
            assert_eq!(at(150.0, 175.0 - 2.0 / zoom), Some(NineSliceGuide::Bottom), "{zoom}");
            // Past the art's box the guides end.
            assert_eq!(at(125.0, 100.0 - 6.0 / zoom), None, "{zoom}");
        }
        let c = cx(&d, &s, &p);
        assert_eq!(guide_at(&c, Point::new(176.0, 173.0)).map(|(_, g)| g), Some(NineSliceGuide::Right));
        assert_eq!(guide_at(&c, Point::new(173.0, 176.0)).map(|(_, g)| g), Some(NineSliceGuide::Bottom));
        assert_eq!(guide_at(&c, Point::new(125.0, 125.0)).map(|(_, g)| g), Some(NineSliceGuide::Left), "left before top");
        assert!(guide_at(&c, Point::new(f64::NAN, 150.0)).is_none());
        let off = editing(None);
        assert!(!over_guide(&cx(&off, &s, &p), Point::new(125.0, 150.0)));
        let mut out = editing(Some(SymbolNineSlice::default()));
        out.symbol_edit = None;
        assert!(!over_guide(&cx(&out, &s, &p), Point::new(125.0, 150.0)));
    }

    /// With the Selection tool, a press on a guide over the art takes the guide, not the art: the
    /// pointer shows the Move cursor, the drag previews that guide alone, and the release commits
    /// it as one step. A click without a drag sends nothing.
    #[test]
    fn dragging_a_guide_moves_that_guide_in_one_step() {
        let d = editing(Some(SymbolNineSlice::default()));
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert_eq!(t.cursor(&c, Point::new(126.0, 150.0), Mods::default()), Cursor::Move);
        assert_ne!(t.cursor(&c, Point::new(150.0, 150.0), Mods::default()), Cursor::Move, "the art between the guides");
        assert!(t.pointer(&c, &ev(PointerKind::Down, 126.0, 150.0)).is_empty(), "the art is not selected");
        assert!(t.busy());
        assert_eq!(t.cursor(&c, Point::new(300.0, 300.0), Mods::default()), Cursor::Move, "kept during the drag");
        let a = t.pointer(&c, &ev(PointerKind::Drag, 110.0, 400.0));
        assert_eq!(a[0], Action::Begin("Symbol Options".into()));
        let (cmd, v) = preview(&a[1]);
        assert_eq!(cmd, "symbol.options");
        assert_eq!(v, json!({ "name": "S", "nineSliceGuides": [0.1, 0.25, 0.25, 0.25] }));
        // Past the opposite guide it stops short of it; modifiers change nothing.
        let shifted = PointerEvent { mods: Mods { shift: true, alt: true, ..Mods::default() }, ..ev(PointerKind::Drag, 190.0, 150.0) };
        let (_, v) = preview(&t.pointer(&c, &shifted)[0]);
        let left = v["nineSliceGuides"][0].as_f64().unwrap();
        assert!((left - (0.75 - vectorcraft_doc::MIN_CENTER_BAND)).abs() < 1e-12, "{v}");
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 190.0, 150.0)), vec![Action::Commit]);
        assert!(!t.busy());
        // A click on a guide without moving.
        assert!(t.pointer(&c, &ev(PointerKind::Down, 175.0, 150.0)).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Drag, 176.0, 150.0)).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, 176.0, 150.0)).is_empty());
        // The top guide moves up and down only.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 125.0));
        let a = t.pointer(&c, &ev(PointerKind::Drag, 10.0, 140.0));
        assert_eq!(preview(&a[1]).1["nineSliceGuides"], json!([0.25, 0.4, 0.25, 0.25]));
    }

    /// Escape during a guide drag cancels it and is the tool's; the release after it commits
    /// nothing. A double-click on a guide neither edits the art nor leaves the mode.
    #[test]
    fn escape_cancels_a_guide_drag_and_a_double_click_stays_on_the_guide() {
        let d = editing(Some(SymbolNineSlice::default()));
        let (s, p) = (Selection::default(), paint());
        let c = ToolContext { isolation: d.symbol_edit.as_ref().map(|e| e.layer), ..cx(&d, &s, &p) };
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 175.0, 150.0));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Drag, 160.0, 150.0))[0], Action::Begin("Symbol Options".into()));
        assert!(t.claims_key(&c, ToolKey::Escape));
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert!(!t.busy() && !t.claims_key(&c, ToolKey::Escape));
        assert!(t.pointer(&c, &ev(PointerKind::Up, 160.0, 150.0)).is_empty());
        // Escape after a click that did not move cancels nothing.
        t.pointer(&c, &ev(PointerKind::Down, 175.0, 150.0));
        assert!(t.key(&c, ToolKey::Escape, Mods::default()).is_empty());
        for k in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
            assert!(t.pointer(&c, &ev(k, 175.0, 150.0)).is_empty(), "{k:?}");
        }
        // Away from the guides a double-click outside the art still leaves isolation.
        assert_eq!(t.pointer(&c, &ev(PointerKind::DoubleClick, 400.0, 400.0)), vec![Action::Exec("object.exitIsolation".into(), json!({}))]);
    }
}
