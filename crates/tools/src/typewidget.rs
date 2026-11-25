//! The type widget: a small circle beside the right side of a selected type object's bounding
//! box, hollow on point type and filled on area type. Double-clicking it converts the type to the
//! other kind (Type › Convert To Area Type / Convert To Point Type), keeping its text and styles.
//! The Selection tool hits it and the canvas draws it from the same geometry.

use serde_json::json;
use vectorcraft_doc::{Document, NodeId, NodeKind, OrientedBox, Selection, TextKind};
use vectorcraft_geom::Point;

use crate::select::selection_box;
use crate::{Action, ToolContext};

/// The widget's centre sits this far (screen px) outside the middle of the box's right side, clear
/// of its handle.
const OFFSET_PX: f64 = 16.0;
/// The widget's radius (screen px), as the canvas draws it.
pub const RADIUS_PX: f32 = 3.5;
/// How near (screen px) the pointer must be to hit it.
const HIT_PX: f64 = 5.0;

/// The type widget of a selection that is exactly one editable point or area type object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeWidget {
    pub id: NodeId,
    /// Centre in document coordinates.
    pub at: Point,
    /// Area type (a filled circle; point type's is hollow).
    pub area: bool,
}

impl TypeWidget {
    /// The widget beside `bx`, the selection's bounding box, at `zoom` (screen px per document
    /// point). None unless the selection is one editable point or area type object (not type on a
    /// path, nor type in perspective, which transforms whole).
    pub fn of(doc: &Document, selection: &Selection, bx: &OrientedBox, zoom: f64) -> Option<Self> {
        let [id] = selection.objects[..] else { return None };
        if !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let n = doc.node(id).filter(|n| n.extras().perspective.is_none())?;
        let NodeKind::Text(t) = &n.kind else { return None };
        let area = match t.kind {
            TextKind::Point => false,
            TextKind::Area { .. } => true,
            _ => return None,
        };
        // Last: on a locked artboard it measures the art of the layer object that holds `id`.
        if !doc.is_editable(id) {
            return None;
        }
        let r = bx.rect;
        Some(Self { id, at: bx.to_doc() * Point::new(r.x1 + OFFSET_PX / zoom, r.center().y), area })
    }

    /// The widget the Selection tool can double-click (its bounding box showing).
    pub fn for_tool(cx: &ToolContext) -> Option<Self> {
        if !cx.show_bbox {
            return None;
        }
        Self::of(cx.doc, cx.selection, &selection_box(cx)?, cx.zoom)
    }

    /// The widget of `cx` if it is under `p`.
    fn at(cx: &ToolContext, p: Point) -> Option<Self> {
        Self::for_tool(cx).filter(|w| w.at.distance(p) <= cx.tol(HIT_PX))
    }

    /// The command converting the type to the other kind.
    pub fn command(&self) -> &'static str {
        if self.area { "type.convertToPointType" } else { "type.convertToAreaType" }
    }
}

/// Is `p` over the type widget?
pub fn over_widget(cx: &ToolContext, p: Point) -> bool {
    TypeWidget::at(cx, p).is_some()
}

/// A double-click on the type widget converts point type to area type and back.
pub fn double_click(cx: &ToolContext, p: Point) -> Option<Action> {
    let w = TypeWidget::at(cx, p)?;
    Some(Action::Exec(w.command().into(), json!({ "ids": [w.id.0] })))
}

#[cfg(test)]
mod tests {
    use vectorcraft_doc::{Node, TextObject};

    use super::*;
    use crate::select::SelectionTool;
    use crate::testutil::*;
    use crate::{Cursor, Mods, PointerEvent, PointerKind, Tool};

    fn ev(kind: PointerKind, p: Point) -> PointerEvent {
        PointerEvent::new(kind, p.x, p.y)
    }

    #[test]
    fn the_widget_sits_right_of_the_box_and_a_double_click_converts() {
        let (mut d, area) = doc_with_area_type();
        let point = d.alloc_id();
        let l = d.layers[0].id;
        let t = TextObject::point(Point::new(100.0, 300.0), "Point", Default::default());
        d.insert(Some(l), 0, Node::new(point, NodeKind::Text(Box::new(t)))).unwrap();
        let p = paint();
        for (id, cmd) in [(area, "type.convertToPointType"), (point, "type.convertToAreaType")] {
            let mut s = Selection::default();
            s.add(id);
            let c = cx(&d, &s, &p);
            let bx = selection_box(&c).unwrap();
            let w = TypeWidget::for_tool(&c).unwrap();
            assert_eq!(w.area, id == area);
            assert!((w.at.x - (bx.rect.x1 + 16.0)).abs() < 1e-9 && (w.at.y - bx.rect.center().y).abs() < 1e-9, "{w:?} {bx:?}");
            // The presses and releases of a double-click change nothing (no marquee deselects).
            let mut t = SelectionTool::default();
            for k in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up] {
                assert_eq!(t.pointer(&c, &ev(k, w.at)), vec![], "{k:?}");
            }
            let a = t.pointer(&c, &ev(PointerKind::DoubleClick, w.at + vectorcraft_geom::Vec2::new(2.0, 0.0)));
            assert_eq!(a, vec![Action::Exec(cmd.into(), json!({ "ids": [id.0] }))]);
            assert_eq!(t.cursor(&c, w.at, Mods::default()), Cursor::TypeWidget);
            // A double-click on the type itself still edits it.
            let inside = bx.center();
            assert_eq!(t.pointer(&c, &ev(PointerKind::DoubleClick, inside)), vec![Action::SwitchTool("type".into())]);
            // Hidden bounding box: no widget.
            let c = ToolContext { show_bbox: false, ..cx(&d, &s, &p) };
            assert!(TypeWidget::for_tool(&c).is_none());
        }
        // Two objects, or type on a path: no widget.
        let mut s = Selection::default();
        s.add(point);
        s.add(area);
        assert!(TypeWidget::for_tool(&cx(&d, &s, &p)).is_none());
        let (pd, path_type) = doc_with_path_type();
        let mut s = Selection::default();
        s.add(path_type);
        assert!(TypeWidget::for_tool(&cx(&pd, &s, &p)).is_none());
    }
}
