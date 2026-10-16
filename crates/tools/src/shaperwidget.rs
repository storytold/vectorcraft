//! The Shaper Group's mode widget: a small arrow beside the right side of the bounds of a Shaper
//! Group's source shapes. It shows while the group or one of its faces is selected (Face Selection
//! mode) and while the group's source shapes are isolated (Construction mode). A click on it
//! switches between the two modes: it isolates the source shapes (`object.isolate`), or it leaves
//! isolation with the group selected (`object.exitIsolation`). The Shaper and Selection tools hit
//! it and the canvas draws it from the same geometry.

use serde_json::json;
use vectorcraft_doc::shaper::SOURCES;
use vectorcraft_doc::{Document, NodeId, Selection};
use vectorcraft_geom::Point;

use crate::{Action, ToolContext};

/// The widget's center sits this far (screen px) outside the middle of the bounds' right side.
const OFFSET_PX: f64 = 16.0;
/// The widget's radius (screen px), as the canvas draws it.
pub const RADIUS_PX: f32 = 6.0;
/// How near (screen px) the pointer must be to hit it.
const HIT_PX: f64 = 7.0;

/// The mode widget of an editable Shaper Group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShaperWidget {
    pub group: NodeId,
    /// Center in document coordinates.
    pub at: Point,
    /// Construction mode: the group's source shapes are isolated.
    pub construction: bool,
}

impl ShaperWidget {
    /// The widget at `zoom` (screen px per document point), beside the bounds of the group's source
    /// shapes in both modes. Construction mode: `isolation` is a Shaper Group's source shapes. Face
    /// Selection mode: the selection is one Shaper Group, or one face or stroke of it. It returns
    /// None otherwise, and when the group, or a layer or group that holds it, is hidden or locked.
    pub fn of(doc: &Document, selection: &Selection, isolation: Option<NodeId>, zoom: f64) -> Option<Self> {
        if !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let shaper = |id: NodeId| doc.node(id).is_some_and(|n| n.extras().shaper.is_some());
        let sources = isolation.filter(|i| doc.node(*i).is_some_and(|n| n.name.as_deref() == Some(SOURCES)));
        let (group, construction) = match sources {
            Some(s) => (doc.parent_of(s).filter(|g| shaper(*g))?, true),
            None => {
                let [id] = selection.objects[..] else { return None };
                let group = if shaper(id) { id } else { doc.parent_of(id).filter(|g| shaper(*g))? };
                (group, false)
            }
        };
        if !doc.is_editable(group) {
            return None;
        }
        let b = doc.node(group)?.children()?.first().filter(|n| n.name.as_deref() == Some(SOURCES))?.geometric_bounds()?;
        Some(Self { group, at: Point::new(b.x1 + OFFSET_PX / zoom, b.center().y), construction })
    }

    /// The widget of `cx` if it is under `p`.
    fn at(cx: &ToolContext, p: Point) -> Option<Self> {
        Self::of(cx.doc, cx.selection, cx.isolation, cx.zoom).filter(|w| w.at.distance(p) <= cx.tol(HIT_PX))
    }

    /// The command switching to the other mode.
    pub fn action(&self) -> Action {
        if self.construction {
            Action::Exec("object.exitIsolation".into(), json!({}))
        } else {
            Action::Exec("object.isolate".into(), json!({ "id": self.group.0 }))
        }
    }
}

/// Is `p` over the Shaper Group's mode widget?
pub fn over_widget(cx: &ToolContext, p: Point) -> bool {
    ShaperWidget::at(cx, p).is_some()
}

/// A click on the mode widget switches between Face Selection and Construction mode.
pub fn click(cx: &ToolContext, p: Point) -> Option<Action> {
    ShaperWidget::at(cx, p).map(|w| w.action())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_doc::shaper::FILL_PREFIX;
    use vectorcraft_doc::{Appearance, Node};
    use vectorcraft_geom::{Rect, shapes};

    use super::*;
    use crate::extra::ShaperTool;
    use crate::select::SelectionTool;
    use crate::testutil::*;
    use crate::{PointerEvent, PointerKind, Tool};

    fn ev(kind: PointerKind, p: Point) -> PointerEvent {
        PointerEvent::new(kind, p.x, p.y)
    }

    /// A Shaper Group of two source squares (x from 0 to 100 and from 50 to 150, y from 300 to 400)
    /// whose visible face is the first square: the document, the group, its source shapes and the
    /// face.
    fn doc_with_shaper_group() -> (Document, NodeId, NodeId, NodeId) {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let square = |x: f64| shapes::rectangle(Rect::new(x, 300.0, x + 100.0, 400.0));
        let a = Node::path(d.alloc_id(), square(0.0), Appearance::default_art());
        let b = Node::path(d.alloc_id(), square(50.0), Appearance::default_art());
        let mut sources = Node::group(d.alloc_id(), vec![Arc::new(a), Arc::new(b)]);
        sources.name = Some(SOURCES.into());
        sources.opacity = 0.0;
        let mut face = Node::path(d.alloc_id(), square(0.0), Appearance::default_art());
        face.name = Some(format!("{FILL_PREFIX}0"));
        let (sources_id, face_id) = (sources.id, face.id);
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(sources), Arc::new(face)]);
        group.extras_mut().shaper = Some(Box::default());
        let group_id = group.id;
        d.insert(Some(l), 1, group).unwrap();
        (d, group_id, sources_id, face_id)
    }

    #[test]
    fn the_widget_sits_right_of_the_group_and_a_click_switches_the_mode() {
        let (d, g, sources, face) = doc_with_shaper_group();
        let p = paint();
        // Face Selection mode: the group, or one of its faces, selected; the widget beside the
        // source shapes' bounds, though the visible face ends at 100.
        for selected in [g, face] {
            let mut s = Selection::default();
            s.add(selected);
            let c = cx(&d, &s, &p);
            let w = ShaperWidget::of(c.doc, c.selection, c.isolation, c.zoom).unwrap();
            assert_eq!((w.group, w.construction, w.at), (g, false, Point::new(166.0, 350.0)));
            let isolate = vec![Action::Exec("object.isolate".into(), json!({ "id": g.0 }))];
            assert_eq!(SelectionTool::default().pointer(&c, &ev(PointerKind::Down, w.at)), isolate);
            assert_eq!(ShaperTool::default().pointer(&c, &ev(PointerKind::Down, w.at + vectorcraft_geom::Vec2::new(0.0, 5.0))), isolate);
            // A double-click on it does nothing more than its two clicks.
            assert_eq!(SelectionTool::default().pointer(&c, &ev(PointerKind::DoubleClick, w.at)), vec![]);
            assert_eq!(ShaperTool::default().pointer(&c, &ev(PointerKind::DoubleClick, w.at)), vec![]);
        }
        // Construction mode: the source shapes isolated; the widget in the same place, and a click
        // leaves the mode.
        let s = Selection::default();
        let c = ToolContext { isolation: Some(sources), ..cx(&d, &s, &p) };
        let w = ShaperWidget::of(c.doc, c.selection, c.isolation, c.zoom).unwrap();
        assert_eq!((w.group, w.construction, w.at), (g, true, Point::new(166.0, 350.0)));
        let exit = vec![Action::Exec("object.exitIsolation".into(), json!({}))];
        assert_eq!(SelectionTool::default().pointer(&c, &ev(PointerKind::Down, w.at)), exit);
        assert_eq!(ShaperTool::default().pointer(&c, &ev(PointerKind::Down, w.at)), exit);
        assert_eq!(SelectionTool::default().pointer(&c, &ev(PointerKind::DoubleClick, w.at)), vec![]);
        assert_eq!(ShaperTool::default().pointer(&c, &ev(PointerKind::DoubleClick, w.at)), vec![]);
        // At 4× zoom the widget sits 4 pt out.
        let c4 = ToolContext { zoom: 4.0, ..c };
        assert_eq!(ShaperWidget::of(c4.doc, c4.selection, c4.isolation, c4.zoom).unwrap().at, Point::new(154.0, 350.0));
    }

    #[test]
    fn only_a_shaper_group_or_its_sources_show_the_widget() {
        let (mut d, g, _, face) = doc_with_shaper_group();
        let rect = d.layers[0].children().unwrap()[0].id;
        let p = paint();
        let widget = |d: &Document, ids: &[NodeId]| {
            let mut s = Selection::default();
            ids.iter().for_each(|id| s.add(*id));
            ShaperWidget::of(d, &s, None, 1.0)
        };
        assert!(widget(&d, &[]).is_none());
        assert!(widget(&d, &[rect]).is_none(), "a plain path");
        assert!(widget(&d, &[rect, g]).is_none(), "two objects");
        assert!(widget(&d, &[g]).is_some());
        // The widget reaches 7 px from its center.
        let mut s = Selection::default();
        s.add(g);
        let c = cx(&d, &s, &p);
        let w = ShaperWidget::of(c.doc, c.selection, c.isolation, c.zoom).unwrap();
        assert!(over_widget(&c, w.at + vectorcraft_geom::Vec2::new(7.0, 0.0)));
        assert!(!over_widget(&c, w.at + vectorcraft_geom::Vec2::new(8.0, 0.0)));
        // A locked group has none.
        d.node_mut(g).unwrap().locked = true;
        assert!(widget(&d, &[g]).is_none());
        assert!(widget(&d, &[face]).is_none());
    }
}
