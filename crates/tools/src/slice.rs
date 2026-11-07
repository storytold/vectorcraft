//! Slice tool (Shift+K) and Slice Selection tool.
//!
//! The Slice tool drags out a user slice (Shift: a square, Alt: from its centre), snapping to the
//! ruler guides and, like the drawing tools, to art: the drag previews `object.slice.create` and
//! commits it as one undo step. The Slice Selection tool clicks a slice to select it (Shift adds or
//! drops it), drags the selected slices to move them (`object.slice.move`: an object slice moves
//! its object), drags a selected user slice's edge or corner handle to resize it
//! (`object.slice.setRect`: what the handle moves snaps as the Slice tool's drag does, Shift keeps
//! the proportions and Alt resizes about the center), opens Slice Options on a double click and
//! deletes the selected slices with Delete. Hidden or locked slices (View menu) can't be picked.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, OrientedBox, SliceSource};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::bbox::{Handle, hit_handle, move_delta, scale_for_drag};
use crate::guides::{PointSnap, Targets};
use crate::shape::drag_rect;
use crate::xform::{BLUE, polygon, rect_corners};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, json_ids};

/// Create a slice tool by id (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "slice" => Box::new(SliceTool::default()),
        "sliceSelection" => Box::new(SliceSelectionTool::default()),
        _ => return None,
    })
}

fn rect_json(r: Rect) -> Value {
    json!({ "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height() })
}

/// A rectangle with a size (a drag along one axis has none).
fn has_area(r: Rect) -> bool {
    r.width() > 1e-6 && r.height() > 1e-6
}

/// Snap `p` as the drawing tools do, then to the nearest ruler guide within reach (guides win).
fn snap(cx: &ToolContext, p: Point) -> (Point, Vec<Overlay>) {
    let (mut q, ov) = crate::guides::snap_draw(cx, p, &[]);
    let tol = cx.tol(5.0);
    let nearest = |vertical: bool, v: f64| {
        cx.doc
            .guides
            .iter()
            .filter(|g| g.vertical == vertical && (g.pos - v).abs() <= tol && cx.doc.guide_shown(g) && cx.doc.guide_passes(g, p, tol))
            .min_by(|a, b| (a.pos - v).abs().total_cmp(&(b.pos - v).abs()))
    };
    if let Some(g) = nearest(true, p.x) {
        q.x = g.pos;
    }
    if let Some(g) = nearest(false, p.y) {
        q.y = g.pos;
    }
    (q, ov)
}

/// The Slice tool: drag to make a user slice.
#[derive(Default)]
pub struct SliceTool {
    start: Option<Point>,
    last: Point,
    began: bool,
    guides: Vec<Overlay>,
}

impl Tool for SliceTool {
    fn id(&self) -> &'static str {
        "slice"
    }

    fn busy(&self) -> bool {
        self.start.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                let (p, _) = snap(cx, ev.pos);
                (self.start, self.last, self.began) = (Some(p), p, false);
                vec![]
            }
            PointerKind::Drag => {
                let Some(start) = self.start else { return vec![] };
                let (p, guides) = snap(cx, ev.pos);
                (self.last, self.guides) = (p, guides);
                let mut out = vec![];
                if !self.began {
                    if p.distance(start) < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    out.push(Action::Begin("Slice".into()));
                }
                let r = drag_rect(start, p, ev.mods);
                if has_area(r) {
                    out.push(Action::Preview("object.slice.create".into(), rect_json(r)));
                }
                out
            }
            PointerKind::Up => {
                self.guides.clear();
                let began = std::mem::take(&mut self.began);
                if self.start.take().is_some() && began { vec![Action::Commit] } else { vec![] }
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.began { self.deactivate(_cx) } else { vec![] }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        match self.start {
            Some(s) if self.began => {
                let d = self.last - s;
                let mut o = self.guides.clone();
                if cx.measurement_labels {
                    o.push(Overlay::Measure { p: self.last, text: cx.size_label(d.x.abs(), d.y.abs()) });
                }
                o
            }
            _ => vec![],
        }
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Slice
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.start = None;
        self.guides.clear();
        if std::mem::take(&mut self.began) { vec![Action::Cancel] } else { vec![] }
    }
}

#[derive(Clone, Debug)]
enum Drag {
    Move { start: Point, ids: Vec<NodeId>, began: bool },
    Resize { id: NodeId, handle: Handle, rect: Rect, began: bool },
}

impl Drag {
    fn began(&self) -> bool {
        match self {
            Drag::Move { began, .. } | Drag::Resize { began, .. } => *began,
        }
    }
}

/// What a user slice's handle snaps to, gathered when it is pressed: what the Slice tool's drag
/// snaps to ([`snap`]), the art through [`PointSnap`] and the ruler guides on their own.
struct ResizeSnap {
    art: PointSnap,
    rulers: Targets,
}

impl ResizeSnap {
    fn new(cx: &ToolContext) -> Self {
        Self { art: PointSnap::new(cx, || Targets::collect(cx.doc, &[], None)), rulers: Targets::rulers(cx) }
    }

    /// Snap the resize `a` ([`scale_for_drag`]) of the slice `rect` by `handle` as [`snap`] snaps
    /// a drawn slice's corner: the handle snaps as a bounding-box handle does
    /// ([`PointSnap::snap_scale`]), then a ruler guide within 5 px of where that leaves it takes
    /// what the handle moves on its axis. Returns the scale and the guides showing why.
    fn scale(&self, cx: &ToolContext, rect: Rect, handle: Handle, a: Affine, m: Mods) -> (Affine, Vec<Overlay>) {
        let bx = OrientedBox::aligned(rect);
        let (a, ov) = self.art.snap_scale(cx, &bx, handle, a, m.shift, m.alt);
        let (b, guide) = self.rulers.snap_scale(&bx, handle, a, m.shift, m.alt, cx.tol(5.0));
        let (r, s) = (a.transform_rect_bbox(rect), b.transform_rect_bbox(rect));
        let moved = [r.x0 - s.x0, r.y0 - s.y0, r.x1 - s.x1, r.y1 - s.y1].iter().any(|d| d.abs() > 1e-9);
        if moved || !guide.is_empty() { (b, guide) } else { (a, ov) }
    }
}

/// The Slice Selection tool: select, move, resize and delete slices.
#[derive(Default)]
pub struct SliceSelectionTool {
    drag: Option<Drag>,
    /// What the user slice handle being dragged snaps to.
    snap: Option<ResizeSnap>,
    /// The guides showing what the resize snapped to.
    guides: Vec<Overlay>,
}

/// Can the slices be picked (not hidden or locked)?
fn pickable(cx: &ToolContext) -> bool {
    !cx.slices_hidden && !cx.slices_locked
}

/// The user or object slice under `p` (the smallest one where several overlap).
fn slice_at(cx: &ToolContext, p: Point) -> Option<NodeId> {
    let lay = cx.doc.slice_layout();
    let hit = lay.iter().filter(|a| a.source != SliceSource::Auto && a.rect.contains(p));
    hit.min_by(|a, b| a.rect.area().total_cmp(&b.rect.area())).and_then(|a| a.id)
}

/// A handle of a selected user slice under `p`: (slice, handle, its rectangle).
fn handle_at(cx: &ToolContext, p: Point) -> Option<(NodeId, Handle, Rect)> {
    cx.selection.slices.iter().filter_map(|id| cx.doc.slice(*id)).find_map(|s| hit_handle(s.rect, p, cx.tol(5.0)).map(|h| (s.id, h, s.rect)))
}

impl SliceSelectionTool {
    /// Select `ids` only.
    fn select(ids: &[NodeId]) -> Action {
        Action::Exec("object.slice.select".into(), json!({ "slices": json_ids(ids) }))
    }
}

impl Tool for SliceSelectionTool {
    fn id(&self) -> &'static str {
        "sliceSelection"
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let (p, m) = (ev.pos, ev.mods);
        match (ev.kind, self.drag.clone()) {
            (PointerKind::Down, _) => {
                self.drag = None;
                self.snap = None;
                self.guides.clear();
                if !pickable(cx) {
                    return vec![];
                }
                if let Some((id, handle, rect)) = handle_at(cx, p) {
                    self.drag = Some(Drag::Resize { id, handle, rect, began: false });
                    self.snap = Some(ResizeSnap::new(cx));
                    return vec![];
                }
                let selected = &cx.selection.slices;
                match slice_at(cx, p) {
                    Some(id) if m.shift => vec![Action::Exec("object.slice.select".into(), json!({ "slices": [id.0], "toggle": true }))],
                    Some(id) if selected.contains(&id) => {
                        self.drag = Some(Drag::Move { start: p, ids: selected.clone(), began: false });
                        vec![]
                    }
                    Some(id) => {
                        self.drag = Some(Drag::Move { start: p, ids: vec![id], began: false });
                        vec![Self::select(&[id])]
                    }
                    None if !m.shift && !selected.is_empty() => vec![Self::select(&[])],
                    None => vec![],
                }
            }
            (PointerKind::Drag, Some(Drag::Move { start, ids, began })) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Move Slice".into()));
                }
                let d = move_delta(start, p, m.shift, cx.constrain_angle);
                out.push(Action::Preview("object.slice.move".into(), json!({ "slices": json_ids(&ids), "dx": d.x, "dy": d.y })));
                self.drag = Some(Drag::Move { start, ids, began: true });
                out
            }
            (PointerKind::Drag, Some(Drag::Resize { id, handle, rect, began })) => {
                let mut out = vec![];
                if !began {
                    out.push(Action::Begin("Resize Slice".into()));
                }
                self.drag = Some(Drag::Resize { id, handle, rect, began: true });
                let mut a = scale_for_drag(rect, handle, p, m.shift, m.alt);
                if let Some(s) = &self.snap {
                    (a, self.guides) = s.scale(cx, rect, handle, a, m);
                }
                let r = a.transform_rect_bbox(rect);
                if has_area(r) {
                    let mut v = rect_json(r);
                    v["id"] = json!(id.0);
                    out.push(Action::Preview("object.slice.setRect".into(), v));
                }
                out
            }
            (PointerKind::Up, Some(d)) => {
                self.drag = None;
                self.snap = None;
                self.guides.clear();
                if d.began() { vec![Action::Commit] } else { vec![] }
            }
            (PointerKind::DoubleClick, _) => {
                self.drag = None;
                self.snap = None;
                self.guides.clear();
                match slice_at(cx, p).filter(|_| pickable(cx)) {
                    Some(id) if cx.selection.slices == [id] => vec![Action::Dialog("sliceOptions".into(), json!({}))],
                    Some(id) => vec![Self::select(&[id]), Action::Dialog("sliceOptions".into(), json!({}))],
                    None => vec![],
                }
            }
            _ => vec![],
        }
    }

    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Delete | ToolKey::Backspace) && !cx.selection.slices.is_empty() && pickable(cx)
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Delete | ToolKey::Backspace if self.drag.is_none() && self.claims_key(cx, key) => {
                vec![Action::Exec("object.slice.delete".into(), json!({}))]
            }
            ToolKey::Escape => self.deactivate(cx),
            _ => vec![],
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = vec![];
        for id in &cx.selection.slices {
            let Some(r) = cx.doc.slice_bounds(*id) else { continue };
            o.push(Overlay::Path { path: polygon(&rect_corners(r), true), color: BLUE, width: 1.0, dashed: false });
            if cx.doc.slice(*id).is_some() && pickable(cx) {
                o.extend(Handle::ALL.iter().map(|h| Overlay::Anchor { p: h.pos(r), color: BLUE, filled: false, size: 7.0 }));
            }
        }
        o.extend(self.guides.iter().cloned());
        // Smart Guides › Transform Tools: W/H while resizing; Measurement Labels: while moving.
        if let Some(Drag::Resize { id, began: true, .. }) = self.drag.as_ref()
            && cx.transform_tools_guides
            && let Some(r) = cx.doc.slice_bounds(*id)
        {
            o.push(Overlay::Measure {
                p: Point::new(r.x1, r.y1) + Vec2::new(cx.tol(12.0), cx.tol(12.0)),
                text: cx.size_label(r.width(), r.height()),
            });
        }
        if let Some(Drag::Move { ids, began: true, .. }) = self.drag.as_ref()
            && cx.measurement_labels
            && let Some(r) = ids.iter().filter_map(|id| cx.doc.slice_bounds(*id)).reduce(|a, b| a.union(b))
        {
            o.push(Overlay::Measure {
                p: Point::new(r.x1, r.y1) + Vec2::new(cx.tol(12.0), cx.tol(12.0)),
                text: cx.size_label(r.width(), r.height()),
            });
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        match handle_at(cx, p).filter(|_| pickable(cx)).map(|(_, h, _)| h) {
            Some(Handle::Top | Handle::Bottom) => Cursor::ResizeV,
            Some(Handle::Left | Handle::Right) => Cursor::ResizeH,
            Some(Handle::TopLeft | Handle::BottomRight) => Cursor::ResizeNwSe,
            Some(Handle::TopRight | Handle::BottomLeft) => Cursor::ResizeNeSw,
            None => Cursor::SliceSelect,
        }
        .turned(cx.view_angle())
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.snap = None;
        self.guides.clear();
        if self.drag.take().is_some_and(|d| d.began()) { vec![Action::Cancel] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Document, Guide, Selection, Slice};

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    fn with(kind: PointerKind, x: f64, y: f64, m: Mods) -> PointerEvent {
        ev(kind, x, y).with_mods(m)
    }

    fn create_preview(a: &[Action]) -> Option<Value> {
        a.iter().find_map(|a| match a {
            Action::Preview(c, v) if c == "object.slice.create" => Some(v.clone()),
            _ => None,
        })
    }

    #[test]
    fn the_slice_tool_drags_out_a_slice_in_one_undo_step() {
        let (mut d, _) = doc_with_rect();
        d.guides.push(Guide::new(true, 403.0));
        let s = Selection::default();
        let p = paint();
        let mut c = cx(&d, &s, &p);
        c.smart_guides = false;
        let mut t = SliceTool::default();
        assert!(t.pointer(&c, &ev(PointerKind::Down, 300.0, 300.0)).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Drag, 300.5, 300.0)).is_empty(), "a jitter is no drag");
        let a = t.pointer(&c, &ev(PointerKind::Drag, 350.0, 320.0));
        assert_eq!(a[0], Action::Begin("Slice".into()));
        assert_eq!(create_preview(&a), Some(json!({"x": 300.0, "y": 300.0, "width": 50.0, "height": 20.0})));
        assert!(matches!(t.overlays(&c).last(), Some(Overlay::Measure { .. })));
        // Shift: a square; Alt: from the centre.
        let sq = t.pointer(&c, &with(PointerKind::Drag, 350.0, 320.0, Mods { shift: true, ..Default::default() }));
        assert_eq!(create_preview(&sq), Some(json!({"x": 300.0, "y": 300.0, "width": 50.0, "height": 50.0})));
        let mid = t.pointer(&c, &with(PointerKind::Drag, 350.0, 320.0, Mods { alt: true, ..Default::default() }));
        assert_eq!(create_preview(&mid), Some(json!({"x": 250.0, "y": 280.0, "width": 100.0, "height": 40.0})));
        // The edge near a guide snaps to it.
        let g = t.pointer(&c, &ev(PointerKind::Drag, 401.0, 320.0));
        assert_eq!(create_preview(&g), Some(json!({"x": 300.0, "y": 300.0, "width": 103.0, "height": 20.0})));
        // A drag along one axis has no area: no preview.
        assert_eq!(create_preview(&t.pointer(&c, &ev(PointerKind::Drag, 350.0, 300.0))), None);
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 350.0, 320.0)), vec![Action::Commit]);
        assert!(!t.busy());
        // A click makes nothing; Escape cancels a drag.
        t.pointer(&c, &ev(PointerKind::Down, 10.0, 10.0));
        assert!(t.pointer(&c, &ev(PointerKind::Up, 10.0, 10.0)).is_empty());
        t.pointer(&c, &ev(PointerKind::Down, 10.0, 10.0));
        t.pointer(&c, &ev(PointerKind::Drag, 40.0, 40.0));
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert_eq!(t.cursor(&c, Point::ZERO, Mods::default()), Cursor::Slice);
    }

    fn doc_with_slices() -> (Document, NodeId, NodeId) {
        let (mut d, _) = doc_with_rect();
        let a = d.alloc_id();
        d.slices.push(Slice { id: a, rect: Rect::new(10.0, 10.0, 60.0, 40.0), options: Default::default() });
        let b = d.alloc_id();
        d.slices.push(Slice { id: b, rect: Rect::new(300.0, 300.0, 400.0, 400.0), options: Default::default() });
        (d, a, b)
    }

    #[test]
    fn the_slice_selection_tool_selects_moves_resizes_and_deletes() {
        let (d, a, b) = doc_with_slices();
        let p = paint();
        let none = Selection::default();
        let c = cx(&d, &none, &p);
        let mut t = SliceSelectionTool::default();
        // A click selects the slice under the pointer; a drag moves it as one gesture.
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 20.0, 20.0)), vec![Action::Exec("object.slice.select".into(), json!({"slices": [a.0]}))]);
        let mut sel = Selection::default();
        sel.set_slices([a]);
        let c = cx(&d, &sel, &p);
        let m = t.pointer(&c, &ev(PointerKind::Drag, 30.0, 25.0));
        assert_eq!(
            m,
            vec![Action::Begin("Move Slice".into()), Action::Preview("object.slice.move".into(), json!({"slices": [a.0], "dx": 10.0, "dy": 5.0}))]
        );
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 30.0, 25.0)), vec![Action::Commit]);
        // Shift-click toggles another slice.
        let shift = Mods { shift: true, ..Default::default() };
        assert_eq!(
            t.pointer(&c, &with(PointerKind::Down, 350.0, 350.0, shift)),
            vec![Action::Exec("object.slice.select".into(), json!({"slices": [b.0], "toggle": true}))]
        );
        t.pointer(&c, &ev(PointerKind::Up, 350.0, 350.0));
        // A corner handle of the selected user slice resizes it.
        assert_eq!(t.cursor(&c, Point::new(60.0, 40.0), Mods::default()), Cursor::ResizeNwSe);
        // In a view turned 90° its cursor turns with the view.
        let mut turned = cx(&d, &sel, &p);
        turned.screen = turned_view(90.0);
        assert_eq!(t.cursor(&turned, Point::new(60.0, 40.0), Mods::default()), Cursor::ResizeNeSw);
        assert!(t.pointer(&c, &ev(PointerKind::Down, 60.0, 40.0)).is_empty());
        let r = t.pointer(&c, &ev(PointerKind::Drag, 80.0, 50.0));
        assert_eq!(r[0], Action::Begin("Resize Slice".into()));
        assert_eq!(r[1], Action::Preview("object.slice.setRect".into(), json!({"id": a.0, "x": 10.0, "y": 10.0, "width": 70.0, "height": 40.0})));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 80.0, 50.0)), vec![Action::Commit]);
        assert!(t.overlays(&c).iter().filter(|o| matches!(o, Overlay::Anchor { .. })).count() == 8);
        // A double click opens Slice Options; Delete deletes the selected slices.
        assert_eq!(t.pointer(&c, &ev(PointerKind::DoubleClick, 20.0, 20.0)), vec![Action::Dialog("sliceOptions".into(), json!({}))]);
        assert!(t.claims_key(&c, ToolKey::Delete));
        assert_eq!(t.key(&c, ToolKey::Delete, Mods::default()), vec![Action::Exec("object.slice.delete".into(), json!({}))]);
        // A click on no slice deselects.
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 200.0, 5.0)), vec![Action::Exec("object.slice.select".into(), json!({"slices": []}))]);
        // Locked slices can't be picked.
        let mut locked = cx(&d, &sel, &p);
        locked.slices_locked = true;
        assert!(t.pointer(&locked, &ev(PointerKind::Down, 20.0, 20.0)).is_empty());
        assert!(!t.claims_key(&locked, ToolKey::Delete));
        assert_eq!(t.cursor(&locked, Point::new(60.0, 40.0), Mods::default()), Cursor::SliceSelect);
    }

    /// The rectangle the last `object.slice.setRect` preview in `a` gives the slice.
    fn set_rect(a: &[Action]) -> Rect {
        let Some(Action::Preview(c, v)) = a.last() else { panic!("{a:?}") };
        assert_eq!(c, "object.slice.setRect");
        let n = |k: &str| v[k].as_f64().unwrap();
        Rect::new(n("x"), n("y"), n("x") + n("width"), n("y") + n("height"))
    }

    fn near(a: Rect, b: Rect) -> bool {
        [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 1e-9)
    }

    /// Drag a handle of slice `a` (10–60 × 10–40, selected) from `from` to `to` with `mods`: the
    /// rectangle the drag previews.
    fn resize(c: &ToolContext, from: (f64, f64), to: (f64, f64), mods: Mods) -> Rect {
        let mut t = SliceSelectionTool::default();
        assert!(t.pointer(c, &with(PointerKind::Down, from.0, from.1, mods)).is_empty());
        set_rect(&t.pointer(c, &with(PointerKind::Drag, to.0, to.1, mods)))
    }

    /// With View › Snap to Grid on, a slice's handle lands what it moves on the nearest gridline
    /// (every 9 pt by default), as the Slice tool's drag and the Selection tool's handles do;
    /// with Snap to Pixel on, on whole pixels.
    #[test]
    fn slice_handles_snap_to_the_grid_and_to_pixels() {
        let (d, a, _) = doc_with_slices();
        let p = paint();
        let mut sel = Selection::default();
        sel.set_slices([a]);
        let grid = ToolContext { snap_to_grid: true, ..cx(&d, &sel, &p) };
        let none = Mods::default();
        // The right side, dragged to x = 64, lands on the gridline at 63.
        assert!(near(resize(&grid, (60.0, 25.0), (64.0, 25.0), none), Rect::new(10.0, 10.0, 63.0, 40.0)));
        // The bottom-right corner, dragged to (64, 46), lands on (63, 45).
        assert!(near(resize(&grid, (60.0, 40.0), (64.0, 46.0), none), Rect::new(10.0, 10.0, 63.0, 45.0)));
        // Shift keeps the proportions, with a moved side on a gridline.
        let r = resize(&grid, (60.0, 40.0), (64.0, 46.0), Mods { shift: true, ..none });
        assert!((r.width() / r.height() - 50.0 / 30.0).abs() < 1e-9 && (r.x0, r.y0) == (10.0, 10.0), "{r:?}");
        assert!([r.x1, r.y1].iter().any(|v| (v / 9.0 - (v / 9.0).round()).abs() < 1e-9), "{r:?}");
        // Alt: the right side lands on 63 and the left side mirrors it about the center (35).
        assert!(near(resize(&grid, (60.0, 25.0), (64.0, 25.0), Mods { alt: true, ..none }), Rect::new(7.0, 10.0, 63.0, 40.0)));
        let pixel = ToolContext { snap_to_pixel: true, ..cx(&d, &sel, &p) };
        assert!(near(resize(&pixel, (60.0, 40.0), (72.4, 50.7), none), Rect::new(10.0, 10.0, 72.0, 51.0)));
    }

    /// A slice's handle snaps to the art as the Slice tool's drag does: with Smart Guides on it
    /// lines up with the other objects' edges, and with them off View › Snap to Point lands it on
    /// an anchor within 2 px. Then a ruler guide within 5 px moves the side or the corner onto
    /// it, with Smart Guides on or off.
    #[test]
    fn slice_handles_snap_to_the_art_and_the_ruler_guides() {
        let (mut d, a, _) = doc_with_slices();
        let p = paint();
        let mut sel = Selection::default();
        sel.set_slices([a]);
        let none = Mods::default();
        let on = cx(&d, &sel, &p);
        // The right side, 3 px from the rectangle's left edge (x = 100), lands on it, and the
        // guide shows until the button goes up.
        let mut t = SliceSelectionTool::default();
        t.pointer(&on, &ev(PointerKind::Down, 60.0, 25.0));
        let r = set_rect(&t.pointer(&on, &ev(PointerKind::Drag, 97.0, 25.0)));
        assert!(near(r, Rect::new(10.0, 10.0, 100.0, 40.0)), "{r:?}");
        assert!(t.overlays(&on).iter().any(|o| matches!(o, Overlay::Line { .. })));
        t.pointer(&on, &ev(PointerKind::Up, 97.0, 25.0));
        assert!(!t.overlays(&on).iter().any(|o| matches!(o, Overlay::Line { .. })));
        // Smart Guides off: Snap to Point lands the corner, 1.4 px from the rectangle's corner, on it.
        let off = ToolContext { smart_guides: false, ..cx(&d, &sel, &p) };
        assert!(near(resize(&off, (60.0, 40.0), (99.0, 101.0), none), Rect::new(10.0, 10.0, 100.0, 100.0)));
        // Snap to Point off too: the side follows the pointer.
        let neither = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &sel, &p) };
        assert!(near(resize(&neither, (60.0, 25.0), (97.0, 25.0), none), Rect::new(10.0, 10.0, 97.0, 40.0)));
        // A ruler guide at x = 98 takes the side dragged 1 px from it, over the rectangle's edge
        // 1 px the other way, with Smart Guides on or off.
        d.guides.push(Guide::new(true, 98.0));
        for c in [cx(&d, &sel, &p), ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &sel, &p) }] {
            assert!(near(resize(&c, (60.0, 25.0), (99.0, 25.0), none), Rect::new(10.0, 10.0, 98.0, 40.0)));
        }
        // With Snap to Grid on, the guide takes the corner's x and the grid still takes its y.
        let grid = ToolContext { snap_to_grid: true, ..cx(&d, &sel, &p) };
        assert!(near(resize(&grid, (60.0, 40.0), (99.0, 46.0), none), Rect::new(10.0, 10.0, 98.0, 45.0)));
    }

    #[test]
    fn slice_resize_readout_honors_transform_tools_guides() {
        let (d, a, _) = doc_with_slices();
        let p = paint();
        let mut sel = Selection::default();
        sel.set_slices([a]);
        let measures = |t: &SliceSelectionTool, c: &ToolContext| t.overlays(c).iter().filter(|o| matches!(o, Overlay::Measure { .. })).count();
        for on in [true, false] {
            let c = ToolContext { transform_tools_guides: on, ..cx(&d, &sel, &p) };
            let mut t = SliceSelectionTool::default();
            t.pointer(&c, &ev(PointerKind::Down, 60.0, 40.0));
            t.pointer(&c, &ev(PointerKind::Drag, 80.0, 50.0));
            assert_eq!(measures(&t, &c), usize::from(on), "transform tools {on}");
            t.pointer(&c, &ev(PointerKind::Up, 80.0, 50.0));
        }
        let move_labels = ToolContext { measurement_labels: true, transform_tools_guides: false, ..cx(&d, &sel, &p) };
        let mut t = SliceSelectionTool::default();
        t.pointer(&move_labels, &ev(PointerKind::Down, 20.0, 20.0));
        t.pointer(&move_labels, &ev(PointerKind::Drag, 30.0, 25.0));
        assert_eq!(measures(&t, &move_labels), 1);
        let no_labels = ToolContext { measurement_labels: false, transform_tools_guides: true, ..cx(&d, &sel, &p) };
        assert_eq!(measures(&t, &no_labels), 0, "move still follows measurement labels");
    }
}
