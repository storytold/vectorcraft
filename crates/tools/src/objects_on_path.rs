//! The Objects on Path tool (#2545): a click on a path attaches the selected objects to it
//! (`object.objectsOnPath.attach`). A path they can attach to is highlighted under the pointer,
//! and the pointer shows [`Cursor::ObjectsOnPath`] over it.
//!
//! Behavior: <https://helpx.adobe.com/illustrator/using/objects-on-path.html>.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_doc::objects_on_path::{attachable_path, is_objects_on_path};
use vectorcraft_geom::{BezPath, Point};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The highlight of the path under the pointer.
const HIGHLIGHT: [u8; 3] = [0x4f, 0x9d, 0xff];

/// The path a click at `p` attaches the selected objects to: the path object hit there, when it can
/// take objects, is in no Objects on Path group, is not a clipping path or part of a compound path,
/// blend, envelope, repeat or Intertwine group ([`vectorcraft_doc::objects_on_path::held`]), and
/// something else is selected.
pub fn target(cx: &ToolContext, p: Point) -> Option<(NodeId, BezPath)> {
    let h = vectorcraft_doc::hit::hit_test(cx.doc, p, cx.hit_options())?;
    let n = cx.doc.node(h.leaf)?;
    let path = attachable_path(n)?;
    if h.ancestry.iter().filter(|a| **a != h.leaf).any(|a| cx.doc.node(*a).is_some_and(is_objects_on_path)) {
        return None;
    }
    if vectorcraft_doc::objects_on_path::held(cx.doc, h.leaf).is_some() {
        return None;
    }
    cx.selection.objects.iter().any(|id| *id != h.leaf).then(|| (h.leaf, path.to_bezpath()))
}

#[derive(Default)]
pub struct ObjectsOnPathTool {
    /// The path under the pointer the selection can attach to, and its outline.
    hover: Option<(NodeId, BezPath)>,
    /// Where the press was, while the button is down.
    press: Option<Point>,
}

impl Tool for ObjectsOnPathTool {
    fn id(&self) -> &'static str {
        "objectsOnPath"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.hover = target(cx, p);
                vec![]
            }
            PointerKind::Down => {
                self.press = Some(p);
                vec![]
            }
            PointerKind::Up => {
                // A press that moved away is not a click.
                let Some(start) = self.press.take().filter(|q| q.distance(p) < cx.tol(3.0)) else { return vec![] };
                let Some((path, _)) = target(cx, start) else { return vec![] };
                self.hover = None;
                vec![Action::Exec("object.objectsOnPath.attach".into(), json!({ "path": path.0 }))]
            }
            _ => vec![],
        }
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        self.hover.iter().map(|(_, path)| Overlay::Path { path: path.clone(), color: HIGHLIGHT, width: 2.0, dashed: false }).collect()
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if target(cx, p).is_some() { Cursor::ObjectsOnPath } else { Cursor::Arrow }
    }

    /// Escape while the button is down cancels the click: the release attaches nothing.
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.press = None;
            self.hover = None;
        }
        vec![]
    }

    fn busy(&self) -> bool {
        self.press.is_some()
    }
}

pub fn create(id: &str) -> Option<Box<dyn crate::Tool>> {
    (id == "objectsOnPath").then(|| Box::new(ObjectsOnPathTool::default()) as Box<dyn crate::Tool>)
}

#[cfg(test)]
mod tests {
    use vectorcraft_doc::{Appearance, Document, Node, NodeKind, Selection};
    use vectorcraft_geom::{Rect, shapes};

    use super::*;
    use crate::testutil::{cx, paint};

    fn doc() -> (Document, NodeId, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let line = d.alloc_id();
        let square = d.alloc_id();
        d.insert(Some(l), 0, Node::path(line, shapes::line(Point::new(0.0, 300.0), Point::new(400.0, 300.0)), Appearance::default_art())).unwrap();
        d.insert(Some(l), 1, Node::path(square, shapes::rectangle(Rect::new(50.0, 50.0, 70.0, 70.0)), Appearance::default_art())).unwrap();
        (d, line, square)
    }

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn a_click_on_a_path_attaches_the_selection_to_it() {
        let (d, line, square) = doc();
        let mut sel = Selection::default();
        sel.set([square]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        let on = Point::new(200.0, 300.0);
        assert_eq!(t.cursor(&cx, on, Mods::default()), Cursor::ObjectsOnPath);
        assert_eq!(t.cursor(&cx, Point::new(200.0, 500.0), Mods::default()), Cursor::Arrow, "away from paths");
        t.pointer(&cx, &ev(PointerKind::Move, on.x, on.y));
        assert_eq!(t.overlays(&cx).len(), 1, "the path is highlighted");
        t.pointer(&cx, &ev(PointerKind::Down, on.x, on.y));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, on.x, on.y));
        assert!(matches!(acts.as_slice(), [Action::Exec(id, p)] if id == "object.objectsOnPath.attach" && p["path"] == line.0), "{acts:?}");
    }

    #[test]
    fn escape_during_the_press_cancels_the_click() {
        let (d, _, square) = doc();
        let mut sel = Selection::default();
        sel.set([square]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&cx, &ev(PointerKind::Move, 200.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 300.0));
        assert!(t.busy());
        t.key(&cx, ToolKey::Escape, Mods::default());
        assert!(!t.busy() && t.overlays(&cx).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 200.0, 300.0)).is_empty(), "the release attaches nothing");
    }

    #[test]
    fn nothing_attaches_without_other_objects_selected() {
        let (d, line, _) = doc();
        let mut sel = Selection::default();
        sel.set([line]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        assert_eq!(t.cursor(&cx, Point::new(200.0, 300.0), Mods::default()), Cursor::Arrow);
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 300.0));
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 200.0, 300.0)).is_empty());
        assert!(matches!(d.node(line).map(|n| &n.kind), Some(NodeKind::Path { .. })));
    }

    #[test]
    fn a_clipping_path_or_a_blend_key_is_not_a_target() {
        let (mut d, line, square) = doc();
        let l = d.layers[0].id;
        let key = d.alloc_id();
        d.insert(Some(l), 2, Node::path(key, shapes::line(Point::new(0.0, 100.0), Point::new(400.0, 100.0)), Appearance::default_art())).unwrap();
        let blend = d.alloc_id();
        d.wrap(&[line, key], |children| Node::new(blend, NodeKind::Blend { children, spec: Default::default() })).unwrap();
        let mask = d.alloc_id();
        let art = d.alloc_id();
        d.insert(Some(l), 2, Node::path(mask, shapes::rectangle(Rect::new(100.0, 380.0, 300.0, 430.0)), Appearance::default_art())).unwrap();
        d.insert(Some(l), 3, Node::path(art, shapes::rectangle(Rect::new(250.0, 390.0, 290.0, 420.0)), Appearance::default_art())).unwrap();
        let clip = d.alloc_id();
        d.wrap(&[mask, art], |children| {
            let mut g = Node::group(clip, children);
            g.set_clips(true);
            g
        })
        .unwrap();
        assert_eq!(d.node(clip).and_then(|g| g.children()).and_then(|c| c.first()).map(|c| c.id), Some(mask));
        let mut sel = Selection::default();
        sel.set([square]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        for (p, what) in [((150.0, 400.0), "a clipping path"), ((200.0, 300.0), "a blend's key path"), ((200.0, 100.0), "a blend's other key path")] {
            assert_eq!(t.cursor(&cx, Point::new(p.0, p.1), Mods::default()), Cursor::Arrow, "{what}");
            t.pointer(&cx, &ev(PointerKind::Move, p.0, p.1));
            assert!(t.overlays(&cx).is_empty(), "{what} is not highlighted");
            t.pointer(&cx, &ev(PointerKind::Down, p.0, p.1));
            assert!(t.pointer(&cx, &ev(PointerKind::Up, p.0, p.1)).is_empty(), "a click on {what} attaches nothing");
        }
    }
}
