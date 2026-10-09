//! The Selection tool (V): click/shift-click, marquee (Shift-drag toggles the objects it reaches:
//! the selected ones are deselected, the others selected), move (Alt copies, Shift constrains; Smart
//! Guides, or with them off View › Snap to Point, snap it),
//! bounding-box scale (Shift proportional, Alt from centre) and rotate (outside corners, Shift 45°),
//! drag a live rectangle's or polygon's corner widget to round its corners (Alt-click cycles their kind,
//! double-click opens the Corners dialog), double-click to enter isolation mode (Double Click To
//! Isolate), Cmd/Ctrl-click to select the object behind (Command Click to Select Objects Behind),
//! click or drag a ruler guide ([`crate::rulerguide`]), drag the brackets of type on a path
//! ([`crate::pathtype`]), double-click the type widget to convert point type to area type and back
//! ([`crate::typewidget`]).
//! The bounding box stands at the selection's own angle after a rotation, so its handles scale
//! along the objects' axes. A handle drag resizes area type's frame (the type area) instead of
//! scaling its type: the text reflows at its size.

use serde_json::{Value, json};
use vectorcraft_doc::hit::{hit_test, marquee, objects_at};
use vectorcraft_doc::{NodeId, OrientedBox};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::bbox::{Handle, hit_handle, in_rotate_zone, move_delta, rotate_for_drag, scale_for_drag};
use crate::corners::{self, CornerDrag, over_widget};
use crate::guides::Targets;
use crate::pathtype::{self, BracketDrag, over_bracket};
use crate::rulerguide::GuideEdit;
use crate::typewidget;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, json_ids};

#[derive(Clone, Debug, Default)]
enum State {
    #[default]
    Idle,
    /// Pressed on an object; becomes Moving after the drag threshold.
    Moving {
        start: Point,
        began: bool,
        /// Shift-pressed on this selected object: released without a drag, it leaves the
        /// selection.
        deselect: Option<NodeId>,
        /// Pressed (no modifier) on this object of a selection of several: released without a
        /// drag, it becomes the key object that Align aligns to (the key again: no key).
        key: Option<NodeId>,
    },
    Scaling {
        handle: Handle,
        bx: OrientedBox,
        /// Area type is selected: its frame resizes (`typeAreas`), its type keeps its size.
        areas: bool,
    },
    Rotating {
        center: Point,
        start: Point,
    },
    Marquee {
        start: Point,
        cur: Point,
        /// Shift: the objects inside leave the selection if selected, else join it.
        toggle: bool,
    },
    /// Dragging a Live Corners widget.
    Corner(CornerDrag),
    /// Dragging a bracket of type on a path.
    Bracket(BracketDrag),
}

#[derive(Default)]
pub struct SelectionTool {
    state: State,
    measure: Option<(Point, String)>,
    guides: Vec<Overlay>,
    /// What a bounding-box handle being dragged snaps to.
    targets: Option<Targets>,
    moving: Option<MoveSnap>,
    guide: GuideEdit,
}

/// Snapping for the selection moved as a whole, its targets gathered when the move begins: View →
/// Snap to Grid lands its bounds on the grid (#740), taking over from the rest; otherwise Smart
/// Guides line its bounds up with the other art, and with them off, View → Snap to Point lands the
/// point it was grabbed by on an anchor or a ruler guide. Snap to Pixel puts its top-left on whole
/// pixels.
pub(crate) struct MoveSnap {
    bounds: Option<Rect>,
    targets: Option<Targets>,
    points: Option<Targets>,
}

impl MoveSnap {
    pub(crate) fn new(cx: &ToolContext) -> Self {
        Self {
            bounds: selection_bounds(cx),
            targets: (cx.smart_guides && !cx.snap_to_grid).then(|| Targets::for_move(cx)),
            points: if cx.snap_to_grid { None } else { Targets::snap_to_point(cx, &cx.selection.objects) },
        }
    }

    /// The move by `d` of the selection grabbed at `start`, snapped, and its guides.
    pub(crate) fn snap(&self, cx: &ToolContext, start: Point, mut d: Vec2) -> (Vec2, Vec<Overlay>) {
        let mut guides = vec![];
        if cx.snap_to_grid
            && let Some(b) = self.bounds
        {
            d += grid_pull(b + d, cx.grid_step());
        }
        if let Some(t) = &self.points {
            let (q, ov) = t.snap_point(start + d, cx.tol(cx.snap_tolerance));
            d = q - start;
            guides = ov;
        }
        if let (Some(t), Some(b)) = (&self.targets, self.bounds) {
            let (adj, ov) = t.snap_rect(b + d, cx.snap_tol());
            d += adj;
            guides = ov;
        }
        if cx.snap_to_pixel
            && let Some(b) = self.bounds
        {
            d = Vec2::new((b.x0 + d.x).round() - b.x0, (b.y0 + d.y).round() - b.y0);
            guides.clear();
        }
        (d, guides)
    }
}

/// The shift that lands `r` on the grid (a gridline every `step`), on each axis by whichever of its
/// two edges and its centre is nearest a gridline, however far that is.
fn grid_pull(r: Rect, step: f64) -> Vec2 {
    let pull = |vs: [f64; 3]| {
        vs.iter().map(|v| vectorcraft_geom::snap::snap_to_grid(*v, step, 0.0) - v).min_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(0.0)
    };
    Vec2::new(pull([r.x0, r.center().x, r.x1]), pull([r.y0, r.center().y, r.y1]))
}

pub fn matrix_json(a: Affine) -> Value {
    let c = a.as_coeffs();
    json!([c[0], c[1], c[2], c[3], c[4], c[5]])
}

/// Selection bounds used for the bounding box: visual bounds with Use Preview Bounds, else
/// geometric bounds.
pub fn selection_bounds(cx: &ToolContext) -> Option<Rect> {
    cx.doc.bounds_of(&cx.selection.objects, cx.preview_bounds)
}

/// [`selection_bounds`] square to the selection's own angle: the bounding box the Selection tool
/// shows and drags (rotated after a rotation).
pub fn selection_box(cx: &ToolContext) -> Option<OrientedBox> {
    cx.doc.oriented_bounds(&cx.selection.objects, cx.preview_bounds)
}

/// What of the bounding box is under the pointer.
enum BoxHit {
    Handle(Handle),
    /// Just outside a corner.
    Rotate,
}

/// The bounding-box handle or rotate zone under `p`.
fn box_hit(cx: &ToolContext, b: &OrientedBox, p: Point) -> Option<BoxHit> {
    let (tol, lp) = (cx.tol(5.0), b.to_local(p));
    if let Some(h) = hit_handle(b.rect, lp, tol) {
        return Some(BoxHit::Handle(h));
    }
    in_rotate_zone(b.rect, lp, tol, cx.tol(18.0)).map(|_| BoxHit::Rotate)
}

impl SelectionTool {
    fn drag_threshold(cx: &ToolContext) -> f64 {
        cx.tol(3.0)
    }
}

impl Tool for SelectionTool {
    fn id(&self) -> &'static str {
        "selection"
    }

    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle) || self.guide.busy()
    }

    fn transforming(&self) -> bool {
        matches!(self.state, State::Moving { began: true, .. } | State::Scaling { .. } | State::Rotating { .. })
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if let Some(out) = self.guide.pointer(cx, ev) {
            return out;
        }
        let p = ev.pos;
        let m = ev.mods;
        match (ev.kind, self.state.clone()) {
            (PointerKind::DoubleClick, _) => {
                self.state = State::Idle;
                if let Some(a) = corners::double_click(cx, p, false).or_else(|| typewidget::double_click(cx, p)) {
                    return vec![a];
                }
                if crate::rulerguide::guide_at(cx, p).is_some() {
                    return vec![];
                }
                if let Some(h) = hit_test(cx.doc, p, cx.hit_options()) {
                    let top = h.top_object(cx.isolation);
                    if cx.double_click_isolate
                        && cx.doc.node(top).is_some_and(|n| {
                            matches!(n.kind, vectorcraft_doc::NodeKind::Group { .. } | vectorcraft_doc::NodeKind::CompoundShape { .. })
                        })
                    {
                        return vec![Action::Exec("object.isolate".into(), json!({ "id": top.0 }))];
                    }
                    if cx.doc.node(top).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Text(_))) {
                        return vec![Action::SwitchTool("type".into())];
                    }
                } else if cx.isolation.is_some() {
                    return vec![Action::Exec("object.exitIsolation".into(), json!({}))];
                }
                vec![]
            }
            (PointerKind::Down, _) => {
                // 1. Live Corners widgets and type on a path's brackets, then the bounding-box
                // handles of the current selection.
                if let Some(c) = CornerDrag::hit(cx, ev, false) {
                    self.state = State::Corner(c);
                    return vec![];
                }
                if let Some(b) = BracketDrag::hit(cx, ev) {
                    self.state = State::Bracket(b);
                    return vec![];
                }
                // The type widget answers a double-click only: its clicks neither drag nor deselect.
                if typewidget::over_widget(cx, p) {
                    self.state = State::Idle;
                    return vec![];
                }
                if cx.show_bbox
                    && let Some(bx) = selection_box(cx)
                {
                    match box_hit(cx, &bx, p) {
                        Some(BoxHit::Handle(handle)) => {
                            let is_area = |id: &NodeId| cx.doc.node(*id).is_some_and(is_area_type);
                            let areas = cx.selection.objects.iter().any(is_area);
                            // Only area type: the step resizes type areas; anything else scales.
                            let label = if areas && cx.selection.objects.iter().all(is_area) { "Resize Type Area" } else { "Scale" };
                            // Smart Guides align what the handle moves with the other objects.
                            self.targets = cx.smart_guides.then(|| Targets::collect(cx.doc, &cx.selection.objects, None).styled(cx));
                            self.state = State::Scaling { handle, bx, areas };
                            return vec![Action::Begin(label.into())];
                        }
                        Some(BoxHit::Rotate) => {
                            self.state = State::Rotating { center: bx.center(), start: p };
                            return vec![Action::Begin("Rotate".into())];
                        }
                        None => {}
                    }
                }
                // 2. Ruler guides (over the art, as they are drawn).
                if let Some(out) = self.guide.press(cx, ev) {
                    self.state = State::Idle;
                    return out;
                }
                // 3. Objects: Cmd/Ctrl-click selects the one under the selected one (cycling).
                if m.cmd
                    && cx.select_behind
                    && let Some(behind) = object_behind(cx, p)
                {
                    self.state = State::Moving { start: p, began: false, deselect: None, key: None };
                    return vec![Action::Exec("select.set".into(), json!({ "ids": [behind.0] }))];
                }
                // A selected compound-shape member drags from anywhere in its own shape.
                if !m.shift
                    && let Some(h) = vectorcraft_doc::hit::selected_member_at(cx.doc, p, cx.hit_options(), &cx.selection.objects)
                {
                    let key = (cx.selection.objects.len() > 1 && !m.cmd && !m.alt).then_some(h.leaf);
                    self.state = State::Moving { start: p, began: false, deselect: None, key };
                    return vec![];
                }
                match hit_test(cx.doc, p, cx.hit_options()) {
                    Some(h) => {
                        let top = h.top_object(cx.isolation);
                        let mut out = vec![];
                        let (mut deselect, mut key) = (None, None);
                        if m.shift {
                            // A Shift-click takes a selected object out of the selection when it
                            // is released; a Shift-drag moves the selection, constrained.
                            if cx.selection.contains(top) {
                                deselect = Some(top);
                            } else {
                                out.push(Action::Exec("select.toggle".into(), json!({ "id": top.0 })));
                            }
                        } else if !cx.selection.contains(top) {
                            out.push(Action::Exec("select.set".into(), json!({ "ids": [top.0] })));
                        } else if cx.selection.objects.len() > 1 && !m.cmd && !m.alt {
                            key = Some(top);
                        }
                        self.state = State::Moving { start: p, began: false, deselect, key };
                        out
                    }
                    None => {
                        self.state = State::Marquee { start: p, cur: p, toggle: m.shift };
                        vec![]
                    }
                }
            }
            (PointerKind::Drag, State::Moving { start, began, .. }) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < Self::drag_threshold(cx) {
                        return out;
                    }
                    out.push(Action::Begin(if m.alt { "Copy".into() } else { "Move".into() }));
                    self.moving = Some(MoveSnap::new(cx));
                }
                let mut d = move_delta(start, p, m.shift);
                if let Some(snap) = &self.moving {
                    (d, self.guides) = snap.snap(cx, start, d);
                }
                self.state = State::Moving { start, began: true, deselect: None, key: None };
                self.measure = cx.measurement_labels.then(|| (p, cx.offset_label(d.x, d.y)));
                out.push(Action::Preview("object.transform".into(), json!({ "matrix": matrix_json(Affine::translate(d)), "copy": m.alt })));
                out
            }
            (PointerKind::Drag, State::Scaling { handle, bx, areas }) => {
                // Scale in the box's own frame: along the objects' axes when it is rotated.
                let mut a = scale_for_drag(bx.rect, handle, bx.to_local(p), m.shift, m.alt);
                self.guides.clear();
                if let Some(t) = &self.targets {
                    (a, self.guides) = t.snap_scale(&bx, handle, a, m.shift, m.alt, cx.snap_tol());
                }
                let nr = a.transform_rect_bbox(bx.rect);
                self.measure = cx.transform_tools_guides.then(|| (p, cx.size_label(nr.width(), nr.height())));
                let mut params = json!({ "matrix": matrix_json(bx.conjugate(a)), "copy": false });
                if areas {
                    params["typeAreas"] = json!(true);
                }
                vec![Action::Preview("object.transform".into(), params)]
            }
            (PointerKind::Drag, State::Rotating { center, start, .. }) => {
                let (a, deg) = rotate_for_drag(center, start, p, m.shift);
                self.measure = cx.transform_tools_guides.then(|| (p, format!("{:.1}°", -deg)));
                vec![Action::Preview("object.transform".into(), json!({ "matrix": matrix_json(a), "copy": false }))]
            }
            (PointerKind::Drag, State::Marquee { start, toggle, .. }) => {
                self.state = State::Marquee { start, cur: p, toggle };
                vec![]
            }
            (PointerKind::Drag, State::Corner(mut c)) => {
                let out = c.drag(cx, p);
                self.state = State::Corner(c);
                out
            }
            (PointerKind::Up, State::Corner(c)) => {
                self.state = State::Idle;
                c.finish()
            }
            (PointerKind::Drag, State::Bracket(mut b)) => {
                let out = b.drag(cx, p, m.cmd);
                self.state = State::Bracket(b);
                out
            }
            (PointerKind::Up, State::Bracket(b)) => {
                self.state = State::Idle;
                b.finish()
            }
            (PointerKind::Up, State::Moving { began, deselect, key, .. }) => {
                self.state = State::Idle;
                self.measure = None;
                self.guides.clear();
                self.moving = None;
                match (began, deselect, key) {
                    (true, ..) => vec![Action::Commit],
                    (false, Some(id), _) => vec![Action::Exec("select.toggle".into(), json!({ "id": id.0 }))],
                    // A click on the key object again: no key.
                    (false, None, Some(id)) if cx.selection.key == Some(id) => vec![Action::Exec("select.key".into(), json!({}))],
                    (false, None, Some(id)) => vec![Action::Exec("select.key".into(), json!({ "id": id.0 }))],
                    (false, None, None) => vec![],
                }
            }
            (PointerKind::Up, State::Scaling { .. } | State::Rotating { .. }) => {
                self.state = State::Idle;
                self.measure = None;
                self.guides.clear();
                self.targets = None;
                vec![Action::Commit]
            }
            (PointerKind::Up, State::Marquee { start, toggle, .. }) => {
                self.state = State::Idle;
                let r = Rect::from_points(start, p);
                if r.width() < Self::drag_threshold(cx) && r.height() < Self::drag_threshold(cx) {
                    return if toggle { vec![] } else { vec![Action::Exec("select.none".into(), json!({}))] };
                }
                let ids: Vec<NodeId> = marquee(cx.doc, r, cx.isolation, false);
                vec![Action::Exec(if toggle { "select.toggle" } else { "select.set" }.into(), json!({ "ids": json_ids(&ids) }))]
            }
            _ => vec![],
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = pathtype::overlays(cx);
        if let Some(source) = cx.isolation.and_then(|id| cx.doc.node(id)).filter(|n| n.name.as_deref() == Some(vectorcraft_doc::shaper::SOURCES)) {
            for n in source.children().into_iter().flatten() {
                if let Some(path) = n.path_data() {
                    o.push(Overlay::Path { path: path.to_bezpath(), color: [0x80, 0x80, 0x80], width: 1.0, dashed: true });
                }
            }
        }
        match &self.state {
            State::Marquee { start, cur, .. } => o.push(Overlay::Marquee(Rect::from_points(*start, *cur))),
            State::Corner(c) => o.extend(c.overlays(cx)),
            _ => {}
        }
        o.extend(self.guides.iter().cloned());
        o.extend(self.guide.overlays(cx));
        if let Some((p, t)) = &self.measure {
            o.push(Overlay::Measure { p: *p, text: t.clone() });
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        match self.state {
            State::Rotating { .. } => return Cursor::Rotate,
            State::Scaling { handle, bx, .. } => return handle_cursor(handle, bx.angle),
            State::Moving { began: true, .. } => return Cursor::Arrow,
            State::Corner(_) => return Cursor::CornerRadius,
            State::Bracket(_) => return Cursor::PathBracket,
            _ => {}
        }
        if self.guide.busy() {
            return self.guide.cursor(cx, p).unwrap_or_default();
        }
        if over_widget(cx, p, false) {
            return Cursor::CornerRadius;
        }
        if over_bracket(cx, p) {
            return Cursor::PathBracket;
        }
        if typewidget::over_widget(cx, p) {
            return Cursor::TypeWidget;
        }
        if cx.show_bbox
            && let Some(bx) = selection_box(cx)
        {
            match box_hit(cx, &bx, p) {
                Some(BoxHit::Handle(h)) => return handle_cursor(h, bx.angle),
                Some(BoxHit::Rotate) => return Cursor::Rotate,
                None => {}
            }
        }
        if let Some(c) = self.guide.cursor(cx, p) {
            return c;
        }
        if vectorcraft_doc::hit::selected_member_at(cx.doc, p, cx.hit_options(), &cx.selection.objects).is_some() {
            return Cursor::Move;
        }
        if let Some(h) = hit_test(cx.doc, p, cx.hit_options()) {
            if cx.selection.contains(h.top_object(cx.isolation)) || m.alt {
                return Cursor::Move;
            }
            return Cursor::Arrow;
        }
        Cursor::Arrow
    }
}

/// Select Behind: of the objects under `p` (topmost first), the one below the lowest selected
/// one, back to the topmost after the bottom one; the topmost when none is selected.
fn object_behind(cx: &ToolContext, p: Point) -> Option<NodeId> {
    let stack = objects_at(cx.doc, p, cx.hit_options(), cx.isolation);
    let next = stack.iter().rposition(|id| cx.selection.contains(*id)).map_or(0, |i| i + 1);
    stack.get(next).or(stack.first()).copied()
}

/// Is `n` area type (text in a frame) whose frame the tools reshape? Type in perspective isn't:
/// it transforms whole.
pub(crate) fn is_area_type(n: &vectorcraft_doc::Node) -> bool {
    n.perspective.is_none() && matches!(&n.kind, vectorcraft_doc::NodeKind::Text(t) if matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }))
}

/// The resize cursor for handle `h` of a box turned by `angle` (counter-clockwise degrees): the
/// handle that sits where `h` appears on screen, in 45° steps.
fn handle_cursor(h: Handle, angle: f64) -> Cursor {
    let steps = if angle.is_finite() { (angle / 45.0).round() as i64 } else { 0 };
    let h = Handle::ALL.get((h as i64 - steps).rem_euclid(8) as usize).copied().unwrap_or(h);
    match h {
        Handle::Top | Handle::Bottom => Cursor::ResizeV,
        Handle::Left | Handle::Right => Cursor::ResizeH,
        Handle::TopLeft | Handle::BottomRight => Cursor::ResizeNwSe,
        Handle::TopRight | Handle::BottomLeft => Cursor::ResizeNeSw,
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
    fn click_selects_object() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        let a = t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
        assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 150.0, 150.0)).is_empty());
    }

    /// #414: a press on a ruler guide picks it over the art, and drags it.
    #[test]
    fn a_press_on_a_ruler_guide_picks_it_over_the_art() {
        let (mut d, _) = doc_with_rect();
        d.guides.push(vectorcraft_doc::Guide::new(false, 150.0));
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert_eq!(t.cursor(&cx, Point::new(160.0, 151.0), Mods::default()), Cursor::ResizeV);
        let a = t.pointer(&cx, &ev(PointerKind::Down, 160.0, 151.0));
        assert_eq!(a, vec![Action::Exec("guide.select".into(), json!({"indexes": [0]}))]);
        assert!(t.busy());
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 171.0));
        assert_eq!(a[1], Action::Preview("guide.move".into(), json!({"dx": 0.0, "dy": 20.0, "copy": false})));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 160.0, 171.0)), vec![Action::Commit]);
        assert!(!t.busy());
        // A double-click there doesn't go through to the art.
        assert!(t.pointer(&cx, &ev(PointerKind::DoubleClick, 160.0, 150.0)).is_empty());
    }

    #[test]
    fn drag_moves_with_single_undo() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0)).is_empty());
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 150.0));
        assert_eq!(a[0], Action::Begin("Move".into()));
        assert!(matches!(&a[1], Action::Preview(c, v) if c == "object.transform" && v["matrix"][4] == 10.0));
        let a = t.pointer(&cx, &ev(PointerKind::Up, 160.0, 150.0));
        assert_eq!(a, vec![Action::Commit]);
    }

    /// #740: with Snap to Grid on, a moved object lands on the grid (every 9 pt by default) by
    /// whichever of its edges or its centre is nearest a gridline, on each axis.
    #[test]
    fn a_move_lands_the_nearest_edge_or_centre_on_the_grid() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = ToolContext { snap_to_grid: true, smart_guides: true, ..cx(&d, &s, &p) };
        let moved = |to: (f64, f64)| {
            let mut t = SelectionTool::default();
            t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
            let a = t.pointer(&cx, &ev(PointerKind::Drag, to.0, to.1));
            match a.get(1) {
                Some(Action::Preview(_, v)) => (v["matrix"][4].as_f64().unwrap(), v["matrix"][5].as_f64().unwrap()),
                other => panic!("{other:?}"),
            }
        };
        // Bounds 100–200 moved by (13, 4): the centres (163, 154) are nearest gridlines (162, 153).
        let (x, y) = moved((163.0, 154.0));
        assert!((x - 12.0).abs() < 1e-9 && (y - 3.0).abs() < 1e-9, "{x} {y}");
        // Moved by (7, 0): the right edge is on a gridline (207) and the top goes up to one (99).
        let (x, y) = moved((157.0, 150.0));
        assert!((x - 7.0).abs() < 1e-9 && (y + 1.0).abs() < 1e-9, "{x} {y}");
    }

    #[test]
    fn shift_drag_on_a_selected_object_moves_it_constrained() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        let shift = |kind, x, y| ev(kind, x, y).with_mods(Mods { shift: true, ..Mods::default() });
        // Pressing keeps the selection: nothing is toggled yet.
        assert!(t.pointer(&cx, &shift(PointerKind::Down, 150.0, 150.0)).is_empty());
        let a = t.pointer(&cx, &shift(PointerKind::Drag, 190.0, 160.0));
        assert_eq!(a[0], Action::Begin("Move".into()));
        assert!(
            matches!(&a[1], Action::Preview(c, v) if c == "object.transform"
                && v["matrix"][4].as_f64().is_some_and(|x| x > 39.0) && v["matrix"][5] == 0.0),
            "moved right, constrained to horizontal: {a:?}"
        );
        assert_eq!(t.pointer(&cx, &shift(PointerKind::Up, 190.0, 160.0)), vec![Action::Commit]);
    }

    #[test]
    fn shift_click_toggles_an_object_on_release() {
        let (d, id) = doc_with_rect();
        let p = paint();
        let shift = |kind, x, y| ev(kind, x, y).with_mods(Mods { shift: true, ..Mods::default() });
        let toggle = vec![Action::Exec("select.toggle".into(), json!({"id": id.0}))];
        // Selected: it leaves the selection when the click is released.
        let mut s = Selection::default();
        s.add(id);
        let cx1 = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert!(t.pointer(&cx1, &shift(PointerKind::Down, 150.0, 150.0)).is_empty());
        assert_eq!(t.pointer(&cx1, &shift(PointerKind::Up, 150.0, 150.0)), toggle);
        // Not selected: it joins the selection at once, ready to be dragged.
        let s = Selection::default();
        let cx2 = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert_eq!(t.pointer(&cx2, &shift(PointerKind::Down, 150.0, 150.0)), toggle);
        assert!(t.pointer(&cx2, &shift(PointerKind::Up, 150.0, 150.0)).is_empty());
    }

    /// #541: a click on one object of a selection of several makes it the key object; a click on
    /// the key again lets it go. A drag moves them all, a lone object has no key.
    #[test]
    fn a_click_on_a_selected_object_makes_it_the_key() {
        let (d, id) = doc_with_area_type();
        let rect = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
        let p = paint();
        let click = |s: &Selection, x, y| {
            let c = cx(&d, s, &p);
            let mut t = SelectionTool::default();
            assert!(t.pointer(&c, &ev(PointerKind::Down, x, y)).is_empty());
            t.pointer(&c, &ev(PointerKind::Up, x, y))
        };
        let mut s = Selection::default();
        s.add(rect);
        s.add(id);
        assert_eq!(click(&s, 150.0, 150.0), vec![Action::Exec("select.key".into(), json!({"id": rect.0}))]);
        s.key = Some(rect);
        assert_eq!(click(&s, 150.0, 150.0), vec![Action::Exec("select.key".into(), json!({}))], "the key again: no key");
        // A drag moves the selection and leaves the key alone.
        let c = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 150.0));
        t.pointer(&c, &ev(PointerKind::Drag, 170.0, 150.0));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 170.0, 150.0)), vec![Action::Commit]);
        // One object selected: a click on it does nothing.
        let mut one = Selection::default();
        one.add(rect);
        assert!(click(&one, 150.0, 150.0).is_empty());
    }

    #[test]
    fn marquee_selects() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 120.0, 120.0));
        assert_eq!(t.overlays(&cx).len(), 1);
        let a = t.pointer(&cx, &ev(PointerKind::Up, 120.0, 120.0));
        assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    }

    /// Shift-drag a marquee: the objects it reaches toggle (#483), so selected ones can be
    /// taken out of the selection; a Shift-click on empty canvas leaves the selection alone.
    #[test]
    fn shift_marquee_toggles() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0).with_mods(shift));
        t.pointer(&cx, &ev(PointerKind::Drag, 120.0, 120.0).with_mods(shift));
        let a = t.pointer(&cx, &ev(PointerKind::Up, 120.0, 120.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("select.toggle".into(), json!({"ids": [id.0]}))]);
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0).with_mods(shift));
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 50.0, 50.0).with_mods(shift)).is_empty());
    }

    #[test]
    fn handle_drag_scales() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert_eq!(t.cursor(&cx, Point::new(200.0, 200.0), Mods::default()), Cursor::ResizeNwSe);
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 200.0, 200.0)), vec![Action::Begin("Scale".into())]);
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 300.0));
        assert!(matches!(&a[0], Action::Preview(_, v) if v["matrix"][0] == 2.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 300.0)), vec![Action::Commit]);
    }

    #[test]
    fn handle_drag_snaps_to_the_other_objects_with_smart_guides() {
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let r = Rect::new(250.0, 100.0, 300.0, 150.0);
        d.insert(Some(l), 1, Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        let height = |a: &[Action]| {
            let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
            v["matrix"][3].as_f64().unwrap() * 50.0
        };
        // The bottom handle, dragged near the neighbour's bottom edge (y = 200), lands on it.
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 275.0, 150.0));
        let a = t.pointer(&c, &ev(PointerKind::Drag, 275.0, 197.0));
        assert!((height(&a) - 100.0).abs() < 1e-9, "{a:?}");
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Line { .. })));
        t.pointer(&c, &ev(PointerKind::Up, 275.0, 197.0));
        assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Line { .. })));
        // Smart Guides off: the edge follows the pointer.
        c.smart_guides = false;
        t.pointer(&c, &ev(PointerKind::Down, 275.0, 150.0));
        let a = t.pointer(&c, &ev(PointerKind::Drag, 275.0, 197.0));
        assert!((height(&a) - 97.0).abs() < 1e-9, "{a:?}");
    }

    /// Smart Guides › Measurement Labels, Transform Tools and Snapping Tolerance (#394): the
    /// readouts while moving and while scaling each follow their option; the tolerance sets how
    /// far a dragged handle is pulled.
    #[test]
    fn smart_guide_display_preferences_drive_the_readouts_and_the_pull() {
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 1, Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(250.0, 100.0, 300.0, 150.0)), Appearance::default_art()))
            .unwrap();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let measures = |t: &SelectionTool, c: &ToolContext| t.overlays(c).iter().filter(|o| matches!(o, Overlay::Measure { .. })).count();
        let height = |a: &[Action]| {
            let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
            v["matrix"][3].as_f64().unwrap() * 50.0
        };
        // Moving: the offset readout is a measurement label.
        for on in [true, false] {
            let c = ToolContext { measurement_labels: on, ..cx(&d, &s, &p) };
            let mut t = SelectionTool::default();
            t.pointer(&c, &ev(PointerKind::Down, 275.0, 125.0));
            t.pointer(&c, &ev(PointerKind::Drag, 285.0, 135.0));
            assert_eq!(measures(&t, &c), usize::from(on), "measurement labels {on}");
            t.pointer(&c, &ev(PointerKind::Up, 285.0, 135.0));
        }
        // Scaling by a handle: the size readout belongs to Transform Tools.
        for on in [true, false] {
            let c = ToolContext { transform_tools_guides: on, ..cx(&d, &s, &p) };
            let mut t = SelectionTool::default();
            t.pointer(&c, &ev(PointerKind::Down, 275.0, 150.0));
            t.pointer(&c, &ev(PointerKind::Drag, 275.0, 180.0));
            assert_eq!(measures(&t, &c), usize::from(on), "transform tools {on}");
            t.pointer(&c, &ev(PointerKind::Up, 275.0, 180.0));
        }
        // Snapping Tolerance: 6 px from the neighbour's bottom edge is beyond 4, within 8.
        for (tol, want) in [(4.0, 94.0), (8.0, 100.0), (0.0, 94.0)] {
            let c = ToolContext { snapping_tolerance: tol, ..cx(&d, &s, &p) };
            let mut t = SelectionTool::default();
            t.pointer(&c, &ev(PointerKind::Down, 275.0, 150.0));
            let a = t.pointer(&c, &ev(PointerKind::Drag, 275.0, 194.0));
            assert!((height(&a) - want).abs() < 1e-9, "tolerance {tol}: {a:?}");
            t.pointer(&c, &ev(PointerKind::Up, 275.0, 194.0));
        }
    }

    #[test]
    fn handle_drag_of_a_turned_box_snaps_on_the_page() {
        use vectorcraft_doc::{Appearance, Node};
        let matrix = |a: &[Action]| {
            let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
            let m: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
            Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]])
        };
        // A 50 pt square beside the 100..200 rect, turned about its centre (275, 125).
        for (turn, low) in [(90.0_f64, 150.0), (45.0, 125.0 + 25.0 * std::f64::consts::SQRT_2)] {
            let (mut d, _) = doc_with_rect();
            let l = d.layers[0].id;
            let id = d.alloc_id();
            let r = Rect::new(250.0, 100.0, 300.0, 150.0);
            d.insert(Some(l), 1, Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art())).unwrap();
            let c = Point::new(275.0, 125.0);
            d.node_mut(id)
                .unwrap()
                .transform(Affine::translate(c.to_vec2()) * Affine::rotate(turn.to_radians()) * Affine::translate(-c.to_vec2()), false);
            let mut s = Selection::default();
            s.add(id);
            let p = paint();
            let cx = cx(&d, &s, &p);
            let bx = selection_box(&cx).unwrap();
            assert_ne!(bx.angle, 0.0);
            // The handle at the lowest point of the shape: a side at 90°, a corner at 45°.
            let grab = Point::new(275.0, low);
            assert!(Handle::ALL.iter().any(|h| (bx.to_doc() * h.pos(bx.rect)).distance(grab) < 1e-6), "{bx:?}");
            let mut t = SelectionTool::default();
            assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, grab.x, grab.y)), vec![Action::Begin("Scale".into())]);
            // Dragged near the neighbour's bottom edge (y = 200), it lands on it.
            let a = t.pointer(&cx, &ev(PointerKind::Drag, 275.0, 197.0));
            let landed = matrix(&a) * grab;
            assert!((landed.y - 200.0).abs() < 1e-6 && (landed.x - 275.0).abs() < 1e-6, "{turn}: {landed:?}");
            assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Line { .. })));
            t.pointer(&cx, &ev(PointerKind::Up, 275.0, 197.0));
        }
    }

    #[test]
    fn handle_drag_on_area_type_resizes_its_frame() {
        let (d, text) = doc_with_area_type();
        let p = paint();
        let mut s = Selection::default();
        s.add(text);
        let cx1 = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        // Bottom-right handle of the 120 × 40 frame at (300, 300).
        assert_eq!(t.pointer(&cx1, &ev(PointerKind::Down, 420.0, 340.0)), vec![Action::Begin("Resize Type Area".into())]);
        let a = t.pointer(&cx1, &ev(PointerKind::Drag, 460.0, 400.0));
        assert!(matches!(&a[0], Action::Preview(c, v) if c == "object.transform" && v["typeAreas"] == true), "{a:?}");
        assert_eq!(t.pointer(&cx1, &ev(PointerKind::Up, 460.0, 400.0)), vec![Action::Commit]);
        // With another object the step is a scale, still resizing the type area.
        let rect = d.layers[0].children().unwrap()[0].id;
        s.add(rect);
        let cx2 = cx(&d, &s, &p);
        assert_eq!(t.pointer(&cx2, &ev(PointerKind::Down, 420.0, 340.0)), vec![Action::Begin("Scale".into())]);
        let a = t.pointer(&cx2, &ev(PointerKind::Drag, 460.0, 400.0));
        assert!(matches!(&a[0], Action::Preview(_, v) if v["typeAreas"] == true), "{a:?}");
        t.pointer(&cx2, &ev(PointerKind::Up, 460.0, 400.0));
        // Without area type the drag scales as before.
        let mut s = Selection::default();
        s.add(rect);
        let cx3 = cx(&d, &s, &p);
        t.pointer(&cx3, &ev(PointerKind::Down, 200.0, 200.0));
        let a = t.pointer(&cx3, &ev(PointerKind::Drag, 300.0, 300.0));
        assert!(matches!(&a[0], Action::Preview(_, v) if v.get("typeAreas").is_none()), "{a:?}");
    }

    #[test]
    fn turned_box_handles_turn_and_scale_along_the_object() {
        let (mut d, id) = doc_with_rect();
        let c = Point::new(150.0, 150.0);
        let turn = Affine::translate(c.to_vec2()) * Affine::rotate(-std::f64::consts::FRAC_PI_4) * Affine::translate(-c.to_vec2());
        d.node_mut(id).unwrap().transform(turn, false);
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        // The Right handle sits up and to the right, where a diagonal cursor fits it.
        let k = 50.0 * std::f64::consts::FRAC_1_SQRT_2;
        let right = Point::new(150.0 + k, 150.0 - k);
        let mut t = SelectionTool::default();
        assert_eq!(t.cursor(&cx, right, Mods::default()), Cursor::ResizeNeSw);
        // The page box's corner is no handle any more.
        assert_ne!(t.cursor(&cx, Point::new(150.0 + 2.0 * k, 150.0 - 2.0 * k), Mods::default()), Cursor::ResizeNeSw);
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, right.x, right.y)), vec![Action::Begin("Scale".into())]);
        let a = t.pointer(&cx, &ev(PointerKind::Drag, right.x + k, right.y - k));
        let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
        // Doubling the width along 45° about the left side: the left handle stays put.
        let m: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let m = Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]]);
        let left = Point::new(150.0 - k, 150.0 + k);
        assert!((m * left).distance(left) < 1e-9);
        assert!((m * right).distance(Point::new(right.x + k, right.y - k)) < 1e-9);
    }

    #[test]
    fn click_empty_deselects() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 400.0, 400.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 400.0, 400.0)), vec![Action::Exec("select.none".into(), json!({}))]);
    }
}
