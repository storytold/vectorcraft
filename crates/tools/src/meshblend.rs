//! Blend tool (W) and Mesh tool (U).
//!
//! - Blend: click object A, then object B → `object.blend.make {ids: [A, B]}`; clicking an anchor
//!   point of a path blends from that point (`starts`). Each further click adds the object as one
//!   more key of the new blend (as does clicking a blend first). Double-click or Alt-click opens
//!   Blend Options (dialog `blendOptions`). Clicking empty canvas starts over. A selected blend
//!   whose spine the steps follow shows Ease and Color handles on its first pair of keys: dragging
//!   them previews `object.blend.options` (Type/Ramp and Shift), one undo step.
//! - Mesh: click inside a filled path → `object.mesh.create {ids, at}` (1×1 mesh plus lines through
//!   the click); click inside a mesh or a mesh envelope → `object.mesh.addLine` (a gradient mesh's
//!   new point takes the current fill color in its own model, unless Shift); drag a mesh point,
//!   or a handle of the point last clicked → `object.mesh.movePoint` previews ([`MeshEdit`]);
//!   Alt-click a mesh point → `object.mesh.deletePoint`. Clicking a gradient mesh's point selects
//!   the mesh, and the next solid fill color recolors just that point.

use serde_json::json;
use vectorcraft_color::Paint;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::Point;

use crate::meshedit::{MeshEdit, grid_of};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext};

const FEEDBACK: [u8; 3] = [0x4f, 0x9d, 0xff];

pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    match id {
        "blend" => Some(Box::new(BlendTool::default())),
        "mesh" => Some(Box::new(MeshTool::default())),
        _ => None,
    }
}

fn hit_top(cx: &ToolContext, p: Point) -> Option<(NodeId, NodeId)> {
    let h = vectorcraft_doc::hit::hit_test(cx.doc, p, cx.hit_options())?;
    Some((h.top_object(cx.isolation), h.leaf))
}

/// The dialog the Blend tool's double-click, Alt-click and toolbar button open.
const BLEND_OPTIONS: &str = "blendOptions";

/// What the Blend tool picks: an object and the anchor of its first subpath clicked, if any.
type Pick = (NodeId, Option<usize>);

#[derive(Default)]
pub struct BlendTool {
    first: Option<Pick>,
    /// A blend was just made: the next click adds to it (the selected blend).
    chained: bool,
    /// A blend handle (steps, ease or colour) being dragged (`blendhandles`).
    drag: Option<crate::blendhandles::BlendDrag>,
}

/// The anchor of the first subpath of path or compound path `id` (its first member's) nearest
/// `p`, within the click tolerance.
fn anchor_at(cx: &ToolContext, id: NodeId, p: Point) -> Option<usize> {
    let n = cx.doc.node(id)?;
    let path = match &n.kind {
        NodeKind::Path { path, .. } => path,
        NodeKind::Compound { children, .. } => children.first()?.path_data()?,
        _ => return None,
    };
    let tol = cx.tol(5.0);
    let d = |a: &vectorcraft_geom::Anchor| a.p.distance(p);
    let (i, a) = path.subpaths.first()?.anchors.iter().enumerate().min_by(|x, y| d(x.1).total_cmp(&d(y.1)))?;
    (d(a) <= tol).then_some(i)
}

/// The object the Blend tool picks at `p` (the top object hit) and the anchor clicked.
fn pick(cx: &ToolContext, p: Point) -> Option<Pick> {
    let (top, _) = hit_top(cx, p)?;
    Some((top, anchor_at(cx, top, p)))
}

impl BlendTool {
    /// The object picked first: the clicked one, or the blend just made.
    fn first(&self, cx: &ToolContext) -> Option<Pick> {
        self.first.or_else(|| match cx.selection.objects.as_slice() {
            [b] if self.chained && cx.doc.node(*b).is_some_and(|n| matches!(n.kind, NodeKind::Blend { .. })) => Some((*b, None)),
            _ => None,
        })
    }
}

impl Tool for BlendTool {
    fn id(&self) -> &'static str {
        "blend"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let options = |t: &mut Self| {
            (t.first, t.chained) = (None, false);
            vec![Action::Dialog(BLEND_OPTIONS.into(), json!({}))]
        };
        match ev.kind {
            PointerKind::DoubleClick => options(self),
            PointerKind::Down if ev.mods.alt => options(self),
            PointerKind::Down => {
                // A selected blend's handles (steps, ease, colour) come before picking objects;
                // the undo step opens with the drag's first change.
                if let Some(d) = crate::blendhandles::BlendDrag::hit_any(cx, ev.pos, ev.mods) {
                    (self.first, self.chained) = (None, false);
                    self.drag = Some(d);
                    return vec![];
                }
                match (self.first(cx), pick(cx, ev.pos)) {
                    (_, None) => {
                        (self.first, self.chained) = (None, false);
                        vec![]
                    }
                    (None, Some((top, start))) => {
                        self.first = Some((top, start));
                        vec![Action::Exec("select.set".into(), json!({"ids": [top.0]}))]
                    }
                    (Some((a, sa)), Some((b, sb))) if a != b && cx.doc.node(a).is_some() => {
                        (self.first, self.chained) = (None, true);
                        let mut p = json!({"ids": [a.0, b.0]});
                        if sa.is_some() || sb.is_some() {
                            p["starts"] = json!([sa, sb]);
                        }
                        vec![Action::Exec("object.blend.make".into(), p)]
                    }
                    (Some(_), Some(_)) => vec![],
                }
            }
            PointerKind::Drag => match self.drag.take() {
                Some(mut d) => {
                    let out = d.drag(ev.pos);
                    self.drag = Some(d);
                    out
                }
                None => vec![],
            },
            PointerKind::Up => self.drag.take().map_or_else(Vec::new, |d| d.finish()),
            _ => vec![],
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        // The drag's label beside the pointer; the handles themselves the canvas draws
        // (`tool_overlays::blend_handles_overlay`).
        if let Some((p, text)) = self.drag.as_ref().and_then(|d| d.label()) {
            out.push(Overlay::Measure { p, text });
        }
        let Some((id, start)) = self.first(cx) else { return out };
        let Some(n) = cx.doc.node(id) else { return out };
        let at = match (start, &n.kind) {
            (Some(i), NodeKind::Path { path, .. }) => path.subpaths.first().and_then(|s| s.anchors.get(i)).map(|a| a.p),
            (Some(i), NodeKind::Compound { children, .. }) => {
                children.first().and_then(|c| c.path_data()).and_then(|p| p.subpaths.first()).and_then(|s| s.anchors.get(i)).map(|a| a.p)
            }
            _ => None,
        };
        let Some(p) = at.or_else(|| n.geometric_bounds().map(|b| b.center())) else { return out };
        out.push(Overlay::Anchor { p, color: FEEDBACK, filled: true, size: 6.0 });
        out
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if let Some(c) = crate::blendhandles::cursor_any(cx, p) {
            return c;
        }
        match pick(cx, p) {
            Some((_, Some(_))) => Cursor::BlendAnchor,
            Some(_) => Cursor::BlendObject,
            None => Cursor::Blend,
        }
    }
    fn busy(&self) -> bool {
        self.first.is_some() || self.drag.is_some()
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        (self.first, self.chained) = (None, false);
        self.drag.take().map_or_else(Vec::new, |d| d.finish())
    }
}

#[derive(Default)]
pub struct MeshTool {
    edit: MeshEdit,
}

impl MeshTool {
    /// Meshes (gradient meshes and mesh envelopes) to consider for point hits: selected ones
    /// first, then any in the document.
    fn meshes(cx: &ToolContext) -> Vec<NodeId> {
        let mut v: Vec<NodeId> = cx.selection.objects.iter().copied().filter(|id| cx.doc.node(*id).and_then(grid_of).is_some()).collect();
        let mut rest = vec![];
        cx.doc.walk(|n| {
            if grid_of(n).is_some() {
                rest.push(n.id);
            }
        });
        for id in rest.into_iter().rev() {
            if !v.contains(&id) {
                v.push(id);
            }
        }
        let editable = cx.doc.editable();
        v.retain(|id| editable(*id));
        v
    }

    fn grab_at(&self, cx: &ToolContext, p: Point) -> Option<crate::meshedit::MeshGrab> {
        self.edit.hit(cx, &Self::meshes(cx), p)
    }
}

impl Tool for MeshTool {
    fn id(&self) -> &'static str {
        "mesh"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Down => {
                if let Some(g) = self.grab_at(cx, p) {
                    if ev.mods.alt && g.handle.is_none() {
                        self.edit.unfocus();
                        return vec![Action::Exec("object.mesh.deletePoint".into(), json!({"id": g.id.0, "index": g.index}))];
                    }
                    // A gradient mesh's point is picked for the next fill colour, so its mesh is
                    // what gets selected.
                    let mut out = vec![];
                    if cx.selection.objects != [g.id] && cx.doc.node(g.id).is_some_and(|n| matches!(n.kind, NodeKind::Mesh(_))) {
                        out.push(Action::Exec("select.set".into(), json!({"ids": [g.id.0]})));
                    }
                    out.extend(self.edit.press(cx, g));
                    return out;
                }
                self.edit.unfocus();
                let Some((top, leaf)) = hit_top(cx, p) else { return vec![] };
                let color = match (&cx.paint.fill, ev.mods.shift) {
                    (Paint::Solid { color, .. }, false) => Some(crate::params::color_json(color)),
                    _ => None,
                };
                for id in [leaf, top] {
                    match cx.doc.node(id).map(|n| &n.kind) {
                        Some(NodeKind::Mesh(_)) => {
                            let mut params = json!({"id": id.0, "x": p.x, "y": p.y});
                            if let Some(c) = &color {
                                params["color"] = c.clone();
                                params["keepModel"] = json!(true);
                            }
                            return vec![Action::Exec("object.mesh.addLine".into(), params)];
                        }
                        Some(NodeKind::Envelope { .. }) if cx.doc.node(id).and_then(grid_of).is_some() => {
                            return vec![Action::Exec("object.mesh.addLine".into(), json!({"id": id.0, "x": p.x, "y": p.y}))];
                        }
                        Some(NodeKind::Path { guide: false, .. }) | Some(NodeKind::Compound { .. }) => {
                            return vec![Action::Exec("object.mesh.create".into(), json!({"ids": [id.0], "at": [p.x, p.y]}))];
                        }
                        _ => {}
                    }
                }
                vec![]
            }
            PointerKind::Drag => self.edit.drag_to(p).unwrap_or_default(),
            PointerKind::Up => self.edit.release().unwrap_or_default(),
            _ => vec![],
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        for id in &cx.selection.objects {
            if let Some(NodeKind::Mesh(m)) = cx.doc.node(*id).map(|n| &n.kind) {
                out.push(Overlay::Path { path: m.lines().to_bezpath(), color: FEEDBACK, width: 1.0, dashed: false });
                for q in &m.points {
                    out.push(Overlay::Anchor { p: q.p, color: FEEDBACK, filled: false, size: 5.0 });
                }
            }
        }
        out.extend(self.edit.overlays(cx));
        out
    }
    fn cursor(&self, cx: &ToolContext, p: Point, mods: Mods) -> Cursor {
        match self.grab_at(cx, p) {
            Some(g) if mods.alt && g.handle.is_none() => Cursor::PenDelete,
            Some(_) => Cursor::Move,
            None => Cursor::MeshAdd,
        }
    }
    fn options(&self) -> serde_json::Value {
        self.edit.options()
    }
    fn busy(&self) -> bool {
        self.edit.dragging()
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.edit.unfocus();
        self.edit.release().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::live::{BlendSpacing, BlendSpec, BlendSpineMode};
    use vectorcraft_doc::{Document, Node, Selection};

    /// Two 20 pt squares blended with Spine Always: the straight spine runs from (10, 10) to
    /// (110, 10), so its handles start at the middle of that line.
    fn doc_with_blend(mode: BlendSpineMode) -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let key = |id: NodeId, x: f64| {
            std::sync::Arc::new(Node::path(
                id,
                vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 0.0, x + 20.0, 20.0)),
                vectorcraft_doc::Appearance::default_art(),
            ))
        };
        let (k1, k2, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let spec = BlendSpec { spacing: BlendSpacing::Steps(3), spine_mode: mode, ..Default::default() };
        let blend = Node::new(b, NodeKind::Blend { children: vec![key(k1, 0.0), key(k2, 100.0)], spec: Box::new(spec) });
        d.insert(Some(l), 0, blend).unwrap();
        (d, b)
    }

    #[test]
    fn blend_tool_drags_the_blend_handles() {
        let (d, b) = doc_with_blend(BlendSpineMode::Always);
        let (mut s, p) = (Selection::default(), paint());
        s.set([b]);
        let c = cx(&d, &s, &p);
        let mut t = create("blend").unwrap();
        // The ease handle on the spine, the linked ramp handle above it, the colour handle below.
        assert_eq!(t.cursor(&c, Point::new(60.0, 10.0), Mods::default()), Cursor::Move);
        assert_eq!(t.cursor(&c, Point::new(60.0, 0.0), Mods::default()), Cursor::Move);
        assert_eq!(t.cursor(&c, Point::new(60.0, 24.0), Mods::default()), Cursor::Move);
        assert_eq!(t.cursor(&c, Point::new(115.0, 5.0), Mods::default()), Cursor::BlendObject);
        // Dragging the ease handle toward the second key eases out; the undo step opens with the
        // drag's first change and the count label travels beside the pointer.
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 60.0, 10.0)), vec![]);
        assert!(t.busy());
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 80.0, 10.0));
        assert!(
            matches!(&acts[..], [Action::Begin(name), Action::Preview(id, p)]
                if name == "Blend Options" && id == "object.blend.options"
                    && p["easingType"] == json!("easeOut")
                    && (p["ramp"].as_f64().unwrap_or_default() - 80.0).abs() < 0.1),
            "{acts:?}"
        );
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "Ease Out 80%")), "{:?}", t.overlays(&c));
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 80.0, 10.0)), vec![Action::Commit]);
        assert!(!t.busy());
        // The linked ramp handle raises the ramp on its own: its distance from the ease knob.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 60.0, 0.0));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 60.0, -16.0));
        assert!(
            matches!(&acts[..], [Action::Begin(_), Action::Preview(id, p)]
                if id == "object.blend.options" && p["easingType"] == json!("easeIn") && p["ramp"] == json!(100.0)),
            "{acts:?}"
        );
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 60.0, -16.0));
        // The colour-shift handle moves independently: 60 % of the way leans the colours late.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 60.0, 24.0));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 70.0, 24.0));
        assert!(
            matches!(&acts[..], [Action::Begin(_), Action::Preview(id, p)]
                if id == "object.blend.options"
                    && (p["colorShift"].as_f64().unwrap_or_default() + 22.02).abs() < 0.01),
            "{acts:?}"
        );
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 70.0, 24.0));
        // The steps handle rides the box's right edge, its height the count: dragging it up adds
        // steps (halfway up the edge is 10), and the label says the count.
        let rest = 20.0 - 20.0 * 3f64.log10() / 2.0;
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 142.0, rest));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 142.0, 10.0));
        assert!(
            matches!(&acts[..], [Action::Begin(_), Action::Preview(id, p)]
                if id == "object.blend.options" && p["spacing"] == json!("steps") && p["steps"] == json!(10)),
            "{acts:?}"
        );
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "10")), "{:?}", t.overlays(&c));
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 142.0, 10.0));
        // Never keeps the easing handles: the easing moves the steps between any two keys.
        let (d2, b2) = doc_with_blend(BlendSpineMode::Never);
        let (mut s2, p2) = (Selection::default(), paint());
        s2.set([b2]);
        let c2 = cx(&d2, &s2, &p2);
        assert_eq!(
            crate::blendhandles::BlendDrag::hit_any(&c2, Point::new(60.0, 10.0), Mods::default()).unwrap().grab(),
            (b2, crate::blendhandles::BlendPart::Ease)
        );
        assert_eq!(t.cursor(&c2, Point::new(60.0, 10.0), Mods::default()), Cursor::Move);
    }

    #[test]
    fn blend_tool_two_clicks_make_blend() {
        let (mut d, a) = doc_with_rect();
        let l = d.layers[0].id;
        let b = d.alloc_id();
        let r = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(300.0, 100.0, 350.0, 150.0));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(b, r, vectorcraft_doc::Appearance::default_art())).unwrap();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = create("blend").unwrap();
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 320.0, 120.0));
        assert_eq!(acts, vec![Action::Exec("object.blend.make".into(), json!({"ids": [a.0, b.0]}))]);
    }

    #[test]
    fn blend_tool_anchor_clicks_and_options() {
        let (mut d, a) = doc_with_rect();
        let l = d.layers[0].id;
        let b = d.alloc_id();
        let r = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(300.0, 100.0, 350.0, 150.0));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(b, r, vectorcraft_doc::Appearance::default_art())).unwrap();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = create("blend").unwrap();
        // Over an anchor, over an object, away from art.
        assert_eq!(t.cursor(&c, Point::new(100.0, 100.0), Mods::default()), Cursor::BlendAnchor);
        assert_eq!(t.cursor(&c, Point::new(150.0, 150.0), Mods::default()), Cursor::BlendObject);
        assert_eq!(t.cursor(&c, Point::new(250.0, 400.0), Mods::default()), Cursor::Blend);
        // The rectangle's first anchor is its top left, b's third its bottom right.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 101.0, 99.0));
        assert!(t.busy());
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 350.0, 150.0));
        assert_eq!(acts, vec![Action::Exec("object.blend.make".into(), json!({"ids": [a.0, b.0], "starts": [0, 2]}))]);
        // Alt-click and double-click open Blend Options.
        let alt = Mods { alt: true, ..Default::default() };
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, 10.0, 10.0).with_mods(alt)),
            vec![Action::Dialog("blendOptions".into(), json!({}))]
        );
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::DoubleClick, 10.0, 10.0)), vec![Action::Dialog("blendOptions".into(), json!({}))]);
    }

    #[test]
    fn blend_tool_next_click_adds_to_the_blend_just_made() {
        let (mut d, a) = doc_with_rect();
        let l = d.layers[0].id;
        let g = d.alloc_id();
        let key = |id: NodeId, x: f64| {
            let r = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 300.0, x + 20.0, 320.0));
            std::sync::Arc::new(vectorcraft_doc::Node::path(id, r, vectorcraft_doc::Appearance::default_art()))
        };
        let (k1, k2) = (d.alloc_id(), d.alloc_id());
        let blend = vectorcraft_doc::Node::new(g, NodeKind::Blend { children: vec![key(k1, 0.0), key(k2, 100.0)], spec: Default::default() });
        d.insert(Some(l), 1, blend).unwrap();
        let mut s = Selection::default();
        let p = paint();
        let mut t = BlendTool { first: None, chained: true, ..Default::default() };
        s.set([g]);
        let c = cx(&d, &s, &p);
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert_eq!(acts, vec![Action::Exec("object.blend.make".into(), json!({"ids": [g.0, a.0]}))]);
        // Away from art: start over.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 450.0, 450.0));
        assert!(t.first(&c).is_none());
    }

    #[test]
    fn mesh_tool_click_path_creates_mesh() {
        let (d, a) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = create("mesh").unwrap();
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 160.0));
        assert_eq!(acts, vec![Action::Exec("object.mesh.create".into(), json!({"ids": [a.0], "at": [150.0, 160.0]}))]);
    }
}
