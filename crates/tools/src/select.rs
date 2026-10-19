//! The Selection tool (V): click/shift-click, marquee (Shift-drag toggles the objects it reaches:
//! the selected ones are deselected, the others selected), move (Alt copies, Shift constrains; Smart
//! Guides, or with them off View › Snap to Point, snap it),
//! bounding-box scale (Shift proportional, Alt from center; Snap to Grid, Snap to Pixel, Smart
//! Guides and Snap to Point snap what the handle moves) and rotate (outside corners, Shift 45°),
//! drag a live rectangle's or polygon's corner widget to round its corners (Alt-click cycles their kind,
//! double-click opens the Corners dialog), drag a live ellipse's pie widgets to set its pie angles
//! (double-click makes it whole again: [`crate::pie`]), double-click a group or an object to enter isolation
//! mode (Double Click To Isolate), Cmd/Ctrl-click to select the object behind (Command Click to Select Objects Behind),
//! click or drag a ruler guide ([`crate::rulerguide`]), Cmd/Ctrl+Shift-double-click a ruler guide or
//! a guide path to release it ([`release_guide_at`]), drag the brackets of type on a path
//! ([`crate::pathtype`]), double-click the type widget to convert point type to area type and back
//! ([`crate::typewidget`]), click a Shaper Group's mode widget to switch between Face Selection and
//! Construction mode ([`crate::shaperwidget`]).
//! The bounding box stands at the selection's own angle after a rotation, so its handles scale
//! along the objects' axes. A handle drag resizes area type's frame (the type area) instead of
//! scaling its type: the text reflows at its size.
//! E pressed during a marquee drag makes the marquee take only the objects entirely inside it, and
//! E pressed again makes it take the objects it touches.
//! A click on the red out port of selected area type whose text overflows loads the pointer with
//! that text ([`Cursor::LoadedText`]): the next click threads it into the closed path or area type
//! under the pointer, or into a new frame of the same shape and size at the click on empty canvas,
//! and a drag draws a rectangle for the new frame (`text.thread.add`). Escape drops the loaded text.

use serde_json::{Value, json};
use vectorcraft_doc::hit::{hit_test, marquee, objects_at};
use vectorcraft_doc::{NodeId, OrientedBox};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::bbox::{Handle, hit_handle, in_rotate_zone, move_delta, rotate_for_drag, scale_for_drag};
use crate::corners::{self, CornerDrag, over_widget};
use crate::guides::{PointSnap, Targets};
use crate::pathtype::{self, BracketDrag, over_bracket};
use crate::pie::{self, PieDrag};
use crate::rulerguide::GuideEdit;
use crate::shaperwidget;
use crate::typewidget;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, json_ids};

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
        /// Pressed (no modifier) on this object of a selection of several, or of one with ruler
        /// guides: released without a drag, it becomes the key object that Align aligns to if
        /// `can_be_key` allows it (the key again: no key).
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
        /// E pressed an odd number of times during the drag: the marquee takes only the objects
        /// that lie entirely inside it.
        enclosed: bool,
    },
    /// Dragging a Live Corners widget.
    Corner(CornerDrag),
    /// Dragging a live ellipse's pie widget.
    Pie(PieDrag),
    /// Move or rotate a live mirror axis.
    Mirror(crate::mirror::MirrorDrag),
    /// Dragging a bracket of type on a path.
    Bracket(BracketDrag),
    /// Loaded with the text of area type `from` by a click on its out port; `press`: where the
    /// next press went down and where the pointer is now.
    Loaded {
        from: NodeId,
        press: Option<(Point, Point)>,
    },
}

#[derive(Default)]
pub struct SelectionTool {
    state: State,
    measure: Option<(Point, String)>,
    guides: Vec<Overlay>,
    /// What a bounding-box handle being dragged snaps to.
    targets: Option<PointSnap>,
    moving: Option<MoveSnap>,
    /// The move snapped to a point or a guide at its last step: the arrow turns hollow.
    snapped: bool,
    guide: GuideEdit,
    /// Smart Guides: the centre or anchor under the pointer, and its label ([`Self::hover_point`]).
    hover: Option<(Point, &'static str)>,
}

/// Snapping for the selection moved as a whole, its targets gathered when the move begins: View →
/// Snap to Grid lands its bounds on the grid (#740), taking over from the rest; otherwise Smart
/// Guides line its bounds up with the other art, and with them off, View → Snap to Point lands the
/// point it was grabbed by (an anchor pressed near, else the pressed point) on an anchor or a ruler
/// guide. Snap to Pixel puts its top-left on whole pixels. A copy (Alt, which can go down or up at
/// any time during the drag) leaves the original behind, so it snaps to the original too.
pub(crate) struct MoveSnap {
    bounds: Option<Rect>,
    targets: Option<Targets>,
    points: Option<Targets>,
    /// The targets and points for a copy, gathered with the others before the previews move the
    /// original.
    copy_targets: Option<Targets>,
    copy_points: Option<Targets>,
    /// With Smart Guides on, the ruler guides View › Snap to Point still pulls the grabbed point
    /// onto (#1534); `points` holds them with Smart Guides off.
    rulers: Option<Targets>,
}

impl MoveSnap {
    pub(crate) fn new(cx: &ToolContext) -> Self {
        let guides = cx.smart_guides && !cx.snap_to_grid;
        let points = |exclude: &[NodeId]| if cx.snap_to_grid { None } else { Targets::snap_to_point(cx, exclude) };
        Self {
            bounds: selection_bounds(cx),
            targets: guides.then(|| Targets::for_move(cx, &cx.selection.objects)),
            points: points(&cx.selection.objects),
            copy_targets: guides.then(|| Targets::for_move(cx, &[])),
            copy_points: points(&[]),
            rulers: (guides && cx.snap_to_point).then(|| Targets::rulers(cx)),
        }
    }

    /// The move (`copy`: the copy) by `d` of the selection grabbed at `start`, snapped, and its
    /// guides.
    pub(crate) fn snap(&self, cx: &ToolContext, start: Point, mut d: Vec2, copy: bool) -> (Vec2, Vec<Overlay>) {
        let (targets, points) = if copy { (&self.copy_targets, &self.copy_points) } else { (&self.targets, &self.points) };
        let mut guides = vec![];
        if cx.snap_to_grid
            && let Some(b) = self.bounds
        {
            d += grid_pull(b + d, cx.grid_step());
        }
        if let Some(t) = points {
            let (q, ov) = t.snap_point(start + d, cx.tol(cx.snap_tolerance));
            d = q - start;
            guides = ov;
        }
        if let (Some(t), Some(b)) = (targets, self.bounds) {
            // The point the selection was grabbed by, near another object's anchor or centre,
            // lands on it, rather than the bounds lining up a little beside it (#955).
            if t.has_point_near(start + d, cx.snap_tol()) {
                let (q, ov) = t.snap_point(start + d, cx.snap_tol());
                d = q - start;
                guides = ov;
            } else {
                let (adj, ov) = t.snap_rect(b + d, cx.snap_tol());
                d += adj;
                guides = ov;
            }
        }
        // A ruler guide within reach of the grabbed point wins over lining the bounds up.
        if let Some(t) = &self.rulers {
            let (q, ov) = t.snap_point(start + d, cx.tol(cx.snap_tolerance));
            if q != start + d {
                d = q - start;
                guides = ov;
            }
        }
        if cx.snap_to_pixel
            && let Some(b) = self.bounds
        {
            d = Vec2::new((b.x0 + d.x).round() - b.x0, (b.y0 + d.y).round() - b.y0);
            guides.clear();
        }
        (d, guides)
    }

    /// [`Self::snap`] for a move Shift keeps to its line (the line through `start` along `d`):
    /// every snap slides the move along that line. With Snap to Grid or Snap to Pixel, the move
    /// slides along the line by the shorter of the two distances that put it on a gridline or a
    /// whole pixel on either axis (the one axis of a level or upright line). The bounds line up
    /// with the other art ([`Targets::snap_rects_along`]) and the grabbed point lands where the line
    /// passes an anchor or crosses a ruler guide ([`Targets::snap_point_along`]). A move of zero
    /// length does not snap.
    pub(crate) fn snap_along(&self, cx: &ToolContext, start: Point, mut d: Vec2, copy: bool) -> (Vec2, Vec<Overlay>) {
        let len = d.hypot();
        if !len.is_finite() || len <= 1e-9 {
            return (d, vec![]);
        }
        let u = d / len;
        // The way along the line that takes away the offset `pull` on one axis: on the axis the
        // line runs along, the shorter way of the two.
        let along = |pull: Vec2| {
            [(pull.x, u.x), (pull.y, u.y)]
                .into_iter()
                .filter(|(_, c)| c.abs() > 1e-9)
                .map(|(p, c)| p / c)
                .min_by(|a, b| a.abs().total_cmp(&b.abs()))
                .unwrap_or(0.0)
        };
        let (targets, points) = if copy { (&self.copy_targets, &self.copy_points) } else { (&self.targets, &self.points) };
        let mut guides = vec![];
        if cx.snap_to_grid
            && let Some(b) = self.bounds
        {
            d += u * along(grid_pull(b + d, cx.grid_step()));
        }
        if let Some(t) = points {
            let (q, ov) = t.snap_point_along(start + d, u, cx.tol(cx.snap_tolerance));
            d = q - start;
            guides = ov;
        }
        if let (Some(t), Some(b)) = (targets, self.bounds) {
            if t.has_point_near(start + d, cx.snap_tol()) {
                let (q, ov) = t.snap_point_along(start + d, u, cx.snap_tol());
                d = q - start;
                guides = ov;
            } else {
                let (adj, ov) = t.snap_rects_along(&[b + d], u, cx.snap_tol());
                d += adj;
                guides = ov;
            }
        }
        if let Some(t) = &self.rulers {
            let (q, ov) = t.snap_point_along(start + d, u, cx.tol(cx.snap_tolerance));
            if q != start + d {
                d = q - start;
                guides = ov;
            }
        }
        if cx.snap_to_pixel
            && let Some(b) = self.bounds
        {
            let at = Point::new(b.x0, b.y0) + d;
            d += u * along(Vec2::new(at.x.round() - at.x, at.y.round() - at.y));
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

/// The objects of `ids` that lie entirely inside marquee `r`, edges included, measured by their
/// geometric bounds with symbol instances measured by their art
/// ([`vectorcraft_doc::Node::geometric_bounds_with`]).
pub(crate) fn inside(cx: &ToolContext, r: Rect, ids: Vec<NodeId>) -> Vec<NodeId> {
    let symbols = cx.doc.symbol_boxes();
    let within = |b: Rect| r.min_x() <= b.min_x() && b.max_x() <= r.max_x() && r.min_y() <= b.min_y() && b.max_y() <= r.max_y();
    ids.into_iter().filter(|id| cx.doc.node(*id).and_then(|n| n.geometric_bounds_with(&symbols)).is_some_and(within)).collect()
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

/// Whether the current selection's bounding-box handle is under `p`, using the same geometry as
/// the Selection tool's press handling.
pub(crate) fn bbox_handle_at(cx: &ToolContext, p: Point) -> bool {
    cx.show_bbox && selection_box(cx).is_some_and(|b| matches!(box_hit(cx, &b, p), Some(BoxHit::Handle(_))))
}

/// How near (screen points) the pointer must come to an object's centre or anchor for Smart Guides
/// to label it while hovering.
const HOVER_REACH: f64 = 6.0;

impl SelectionTool {
    fn drag_threshold(cx: &ToolContext) -> f64 {
        cx.tol(3.0)
    }

    /// The anchor of an object near `p` (the one under the pointer, else a selected one when a
    /// press would keep the selection), or with Smart Guides on its centre, and its label,
    /// "anchor" or "center", as Illustrator shows them while hovering with Smart Guides (#812). A
    /// press there grabs the object by that point, so with Smart Guides or View → Snap to Point
    /// that point is the one that snaps (#1274).
    fn hover_point(cx: &ToolContext, p: Point) -> Option<(Point, &'static str)> {
        if !cx.smart_guides && !cx.snap_to_point {
            return None;
        }
        let reach = cx.tol(HOVER_REACH);
        let under = hit_test(cx.doc, p, cx.hit_options()).map(|h| h.top_object(cx.isolation));
        let nearest = |points: &mut dyn Iterator<Item = Point>| {
            points.filter(|q| q.distance(p) <= reach).min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
        };
        // A press on an unselected object selects it alone: the selected ones stay behind.
        let selected = if under.is_none_or(|u| cx.selection.contains(u)) { &cx.selection.objects[..] } else { &[] };
        under.into_iter().chain(selected.iter().copied()).find_map(|id| {
            let n = cx.doc.node(id)?;
            let anchor = n.path_data().and_then(|pd| nearest(&mut pd.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p))));
            anchor.map(|q| (q, "anchor")).or_else(|| {
                if !cx.smart_guides {
                    return None;
                }
                let c = cx.doc.bounds_of(&[id], cx.preview_bounds)?.center();
                (c.distance(p) <= reach).then_some((c, "center"))
            })
        })
    }

    /// The pointer loaded with the text of area type `from` ([`State::Loaded`]): a press starts
    /// placing it, and its release threads it (`text.thread.add`). None when `from` is no longer
    /// area type: the press is an ordinary one.
    fn loaded(&mut self, cx: &ToolContext, ev: &PointerEvent, from: NodeId, press: Option<(Point, Point)>) -> Option<Vec<Action>> {
        let p = ev.pos;
        match (ev.kind, press) {
            (PointerKind::Down, _) if !cx.doc.node(from).is_some_and(is_area_type) => {
                self.state = State::Idle;
                return None;
            }
            (PointerKind::Down, _) => self.state = State::Loaded { from, press: Some((p, p)) },
            (PointerKind::Drag, Some((start, _))) => self.state = State::Loaded { from, press: Some((start, p)) },
            (PointerKind::Up, Some((start, _))) => {
                self.state = State::Idle;
                return Some(vec![Action::Exec("text.thread.add".into(), thread_params(cx, from, start, p))]);
            }
            _ => {}
        }
        Some(vec![])
    }
}

/// Where the text of `from` goes for a press at `start` released at `end`: a dragged rectangle,
/// else the closed path or area type pressed on, else a new frame with its top-left at `start`.
fn thread_params(cx: &ToolContext, from: NodeId, start: Point, end: Point) -> Value {
    let r = Rect::from_points(start, end);
    if r.width() > cx.tol(6.0) && r.height() > cx.tol(6.0) {
        return json!({ "from": from.0, "rect": [r.x0, r.y0, r.width(), r.height()] });
    }
    let frame = |n: &vectorcraft_doc::Node| {
        is_area_type(n) || matches!(&n.kind, vectorcraft_doc::NodeKind::Path { path, .. } if path.subpaths.first().is_some_and(|s| s.closed))
    };
    let to =
        hit_test(cx.doc, start, cx.hit_options()).map(|h| h.top_object(cx.isolation)).filter(|id| *id != from && cx.doc.node(*id).is_some_and(frame));
    match to {
        Some(to) => json!({ "from": from.0, "to": to.0 }),
        None => json!({ "from": from.0, "at": [start.x, start.y] }),
    }
}

impl Tool for SelectionTool {
    fn id(&self) -> &'static str {
        "selection"
    }

    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle) || self.guide.busy()
    }

    /// E during a marquee drag switches the marquee between the objects it touches and only the
    /// objects inside it.
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Letter('e')
            && let State::Marquee { enclosed, .. } = &mut self.state
        {
            *enclosed = !*enclosed;
            return vec![];
        }
        if key == ToolKey::Escape
            && matches!(self.state, State::Mirror(_))
            && let State::Mirror(d) = std::mem::take(&mut self.state)
        {
            return d.cancel();
        }
        if key == ToolKey::Escape && matches!(self.state, State::Loaded { .. }) {
            self.state = State::Idle;
            return vec![];
        }
        if key == ToolKey::Escape { self.guide.cancel() } else { vec![] }
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        match self.state {
            State::Marquee { .. } => key == ToolKey::Letter('e'),
            State::Loaded { .. } | State::Mirror(_) => key == ToolKey::Escape,
            _ => false,
        }
    }

    fn transforming(&self) -> bool {
        match &self.state {
            State::Moving { began: true, .. } | State::Scaling { .. } | State::Rotating { .. } => true,
            State::Corner(c) => c.adjusting(),
            State::Pie(d) => d.adjusting(),
            State::Bracket(b) => b.adjusting(),
            State::Mirror(d) => d.adjusting(),
            _ => false,
        }
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if let State::Loaded { from, press } = self.state
            && let Some(out) = self.loaded(cx, ev, from, press)
        {
            return out;
        }
        // Mirror controls take precedence over art, bounding handles and ruler guides.
        if let State::Mirror(mut d) = self.state.clone() {
            match ev.kind {
                PointerKind::Drag | PointerKind::Up => {
                    let mut out = d.drag(cx, ev.pos, ev.mods);
                    if ev.kind == PointerKind::Up {
                        out.extend(d.finish());
                        self.state = State::Idle;
                    } else {
                        self.state = State::Mirror(d);
                    }
                    return out;
                }
                _ => return vec![],
            }
        }
        if matches!(self.state, State::Idle)
            && ev.kind == PointerKind::Down
            && let Some(d) = crate::mirror::MirrorDrag::hit(cx, ev.pos, ev.mods)
        {
            self.state = State::Mirror(d);
            self.hover = None;
            return vec![];
        }
        if let Some(out) = self.guide.pointer(cx, ev) {
            return out;
        }
        let p = ev.pos;
        let m = ev.mods;
        // A press near a centre or an anchor grabs the object by it.
        self.hover = None;
        let grab = if matches!(ev.kind, PointerKind::Down) { Self::hover_point(cx, p).map_or(p, |(q, _)| q) } else { p };
        if matches!(ev.kind, PointerKind::Move) && matches!(self.state, State::Idle) {
            self.hover = Self::hover_point(cx, p);
        }
        match (ev.kind, self.state.clone()) {
            (PointerKind::DoubleClick, _) => {
                self.state = State::Idle;
                if let Some(a) = corners::double_click(cx, p, false).or_else(|| pie::double_click(cx, p)).or_else(|| typewidget::double_click(cx, p))
                {
                    return vec![a];
                }
                // The Shaper Group's mode widget answers each click of a double-click on it.
                if shaperwidget::over_widget(cx, p) {
                    return vec![];
                }
                if let Some(a) = release_guide_at(cx, p, m) {
                    return vec![a];
                }
                if crate::rulerguide::guide_at(cx, p).is_some() {
                    return vec![];
                }
                if let Some(h) = hit_test(cx.doc, p, cx.hit_options()) {
                    let top = h.top_object(cx.isolation);
                    if cx.doc.node(top).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Text(_))) {
                        return vec![Action::SwitchTool("type".into())];
                    }
                    // An object that [`isolates`], not the one already isolated.
                    if cx.double_click_isolate && cx.doc.node(top).is_some_and(|n| isolates(&n.kind)) && cx.isolation != Some(top) {
                        return vec![Action::Exec("object.isolate".into(), json!({ "id": top.0 }))];
                    }
                } else if cx.isolation.is_some() {
                    return vec![Action::Exec("object.exitIsolation".into(), json!({}))];
                }
                vec![]
            }
            (PointerKind::Down, _) => {
                // 0. The out port of selected area type: the pointer takes its text.
                if let Some(from) = crate::text::out_port_at(cx, p) {
                    self.state = State::Loaded { from, press: None };
                    return vec![];
                }
                // 1. Live Corners widgets, pie widgets and type on a path's brackets, then the
                // bounding-box handles of the current selection.
                if let Some(c) = CornerDrag::hit(cx, ev, false) {
                    self.state = State::Corner(c);
                    return vec![];
                }
                if let Some(d) = PieDrag::hit(cx, ev) {
                    self.state = State::Pie(d);
                    return vec![];
                }
                if let Some(b) = BracketDrag::hit(cx, ev) {
                    self.state = State::Bracket(b);
                    return vec![];
                }
                if let Some(a) = shaperwidget::click(cx, p) {
                    self.state = State::Idle;
                    return vec![a];
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
                            // What the handle moves snaps as a dragged point does: to pixels or the
                            // grid, else with Smart Guides to the other objects, else with Snap to
                            // Point to their anchors; the ruler guides with Snap to Point.
                            self.targets = Some(PointSnap::new(cx, || Targets::for_scale(cx)));
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
                        } else if (cx.selection.objects.len() > 1 || !cx.selection.guides.is_empty()) && !m.cmd && !m.alt {
                            key = Some(top);
                        }
                        self.state = State::Moving { start: grab, began: false, deselect, key };
                        out
                    }
                    None => {
                        self.state = State::Marquee { start: p, cur: p, toggle: m.shift, enclosed: false };
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
                let unsnapped = d;
                if let Some(snap) = &self.moving {
                    (d, self.guides) = if m.shift { snap.snap_along(cx, start, d, m.alt) } else { snap.snap(cx, start, d, m.alt) };
                }
                // Shift has already constrained `d`: any change is a snap.
                self.snapped = (d - unsnapped).hypot() > 1e-9;
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
                    (a, self.guides) = t.snap_scale(cx, &bx, handle, a, m.shift, m.alt);
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
            (PointerKind::Drag, State::Marquee { start, toggle, enclosed, .. }) => {
                self.state = State::Marquee { start, cur: p, toggle, enclosed };
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
            (PointerKind::Drag, State::Pie(mut d)) => {
                let out = d.drag(cx, p);
                self.state = State::Pie(d);
                out
            }
            (PointerKind::Up, State::Pie(d)) => {
                self.state = State::Idle;
                d.finish()
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
                    (false, None, Some(id)) if can_be_key(cx, id) => vec![Action::Exec("select.key".into(), json!({ "id": id.0 }))],
                    (false, None, _) => vec![],
                }
            }
            (PointerKind::Up, State::Scaling { .. } | State::Rotating { .. }) => {
                self.state = State::Idle;
                self.measure = None;
                self.guides.clear();
                self.targets = None;
                vec![Action::Commit]
            }
            (PointerKind::Up, State::Marquee { start, toggle, enclosed, .. }) => {
                self.state = State::Idle;
                let r = Rect::from_points(start, p);
                if r.width() < Self::drag_threshold(cx) && r.height() < Self::drag_threshold(cx) {
                    return if toggle { vec![] } else { vec![Action::Exec("select.none".into(), json!({}))] };
                }
                let touched = marquee(cx.doc, r, cx.isolation, false);
                let ids = if enclosed { inside(cx, r, touched) } else { touched };
                let art = Action::Exec(if toggle { "select.toggle" } else { "select.set" }.into(), json!({ "ids": json_ids(&ids) }));
                std::iter::once(art).chain(crate::rulerguide::marquee_select(cx, r, enclosed)).collect()
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
            State::Pie(d) => o.extend(d.overlays(cx)),
            State::Loaded { press: Some((start, cur)), .. } => o.push(Overlay::Marquee(Rect::from_points(*start, *cur))),
            _ => {}
        }
        o.extend(self.guides.iter().cloned());
        o.extend(self.guide.overlays(cx));
        if let (Some((q, label)), State::Idle, true) = (self.hover, &self.state, cx.smart_guides) {
            let style = crate::guides::GuideStyle::of(cx);
            if style.labels {
                o.push(Overlay::Label { p: q, text: label.into(), color: style.color });
            }
        }
        if let Some((p, t)) = &self.measure {
            o.push(Overlay::Measure { p: *p, text: t.clone() });
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        if let State::Mirror(d) = &self.state {
            return d.cursor(cx, p);
        }
        if matches!(self.state, State::Idle)
            && !m.cmd
            && !m.ctrl
            && !m.alt
            && let Some(cursor) = crate::mirror::cursor(cx, p)
        {
            return cursor;
        }
        match self.state {
            State::Rotating { center, .. } => return Cursor::rotate_about_in_view(center, p, cx.view_angle()),
            State::Scaling { handle, bx, .. } => return handle_cursor(handle, bx.angle - cx.view_angle()),
            // Alt-dragging leaves a copy behind; otherwise the filled arrow, hollow while the move
            // snaps to a point or a guide.
            State::Moving { began: true, .. } if m.alt => return Cursor::SelectionCopy,
            State::Moving { began: true, .. } => return if self.snapped { Cursor::ArrowHollow } else { Cursor::Arrow },
            State::Corner(_) => return Cursor::CornerRadius,
            State::Pie(_) => return Cursor::ArrowHollow,
            State::Bracket(_) => return Cursor::PathBracket,
            State::Loaded { .. } => return Cursor::LoadedText,
            _ => {}
        }
        if self.guide.busy() {
            return self.guide.cursor(cx, p).unwrap_or_default();
        }
        if over_widget(cx, p, false) {
            return Cursor::CornerRadius;
        }
        if pie::over_widget(cx, p) {
            return Cursor::ArrowHollow;
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
                Some(BoxHit::Handle(h)) => return handle_cursor(h, bx.angle - cx.view_angle()),
                Some(BoxHit::Rotate) => return Cursor::rotate_about_in_view(bx.center(), p, cx.view_angle()),
                None => {}
            }
        }
        if let Some(c) = self.guide.cursor(cx, p) {
            return c;
        }
        // View → Snap to Point: over an anchor a press would grab the object by, the arrow turns
        // hollow.
        if cx.snap_to_point && matches!(Self::hover_point(cx, p), Some((_, "anchor"))) {
            return Cursor::ArrowHollow;
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

/// Can a click make `id` the key object? `select.key` takes only a selected top-level object. In
/// isolation mode a click can pick a sublayer (inside an isolated layer), a compound-path member
/// (inside an isolated compound path) or an object inside another selected object (inside an
/// isolated group that is also selected), and `select.key` returns an error for all three.
fn can_be_key(cx: &ToolContext, id: NodeId) -> bool {
    let inside_selected = cx.doc.ancestry(id).unwrap_or_default().iter().any(|a| *a != id && cx.selection.contains(*a));
    // `paint_owner` returns a compound-path member's compound path.
    let member = crate::xform::paint_owner(cx.doc, id) != id;
    cx.doc.node(id).is_some_and(|n| !n.is_layer()) && !inside_selected && !member
}

/// Is `n` area type (text in a frame) whose frame the tools reshape? Type in perspective isn't:
/// it transforms whole.
pub(crate) fn is_area_type(n: &vectorcraft_doc::Node) -> bool {
    n.extras().perspective.is_none()
        && matches!(&n.kind, vectorcraft_doc::NodeKind::Text(t) if matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }))
}

/// Does a double-click or the Control bar's Isolate Selected Object button isolate an object of
/// this kind? A group, a compound path or a single path, image, mesh or placed file does. Blends,
/// envelopes, repeats and symbol instances do not; they have editing modes of their own.
pub fn isolates(kind: &vectorcraft_doc::NodeKind) -> bool {
    use vectorcraft_doc::NodeKind as K;
    matches!(kind, K::Group { .. } | K::Compound { .. } | K::Path { .. } | K::Image(_) | K::Mesh(_) | K::PlacedDocument(_))
}

/// A Cmd/Ctrl+Shift double-click at `p` with a selection tool releases the guide there: the ruler
/// guide under the pointer (`view.guides.release {index}`), else the guide path hit there
/// (`view.guides.release {ids}`). It returns None without both keys, while guides are hidden or
/// locked, or when no guide is at `p`.
pub(crate) fn release_guide_at(cx: &ToolContext, p: Point, m: Mods) -> Option<Action> {
    if !(m.cmd && m.shift && cx.guides) {
        return None;
    }
    if let Some(i) = crate::rulerguide::guide_at(cx, p) {
        return Some(Action::Exec("view.guides.release".into(), json!({ "index": i })));
    }
    let leaf = hit_test(cx.doc, p, cx.hit_options())?.leaf;
    let guide = cx.doc.node(leaf).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Path { guide: true, .. }));
    guide.then(|| Action::Exec("view.guides.release".into(), json!({ "ids": [leaf.0] })))
}

/// The resize cursor for handle `h` of a box turned by `angle` (counter-clockwise degrees): the
/// handle that sits where `h` appears on screen, in 45° steps. In a turned view, `angle` is the
/// box's own angle less [`ToolContext::view_angle`].
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

    /// #955: grabbed by an anchor and dropped near another object's anchor, the anchor lands
    /// exactly on it with Smart Guides, even where lining the bounds up would leave it beside it.
    #[test]
    fn a_move_grabbed_by_an_anchor_lands_it_on_another_anchor() {
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        let tri = d.alloc_id();
        let pts = [Point::new(350.3, 250.0), Point::new(300.0, 320.0), Point::new(420.0, 330.0)];
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, true));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(tri, path, vectorcraft_doc::Appearance::default_art())).unwrap();
        let (none, mut picked) = (Selection::default(), Selection::default());
        picked.add(id);
        let p = paint();
        let mut t = SelectionTool::default();
        // Grab the rectangle (not selected yet: no bounding box) by its bottom-right corner
        // (200, 200); the press selects it, then drop it beside the tip.
        let before = cx(&d, &none, &p);
        t.pointer(&before, &ev(PointerKind::Move, 201.0, 201.0));
        t.pointer(&before, &ev(PointerKind::Down, 201.0, 201.0));
        let a = t.pointer(&cx(&d, &picked, &p), &ev(PointerKind::Drag, 350.0, 249.8));
        let Some(Action::Preview(_, v)) = a.get(1) else { panic!("{a:?}") };
        let (x, y) = (v["matrix"][4].as_f64().unwrap(), v["matrix"][5].as_f64().unwrap());
        assert!((200.0 + x - 350.3).abs() < 1e-9 && (200.0 + y - 250.0).abs() < 1e-9, "the corner lands at ({}, {})", 200.0 + x, 200.0 + y);
    }

    /// A corner-radius drag is a transform, so the sharp selection box hides while the radius changes.
    #[test]
    fn a_corner_drag_counts_as_transforming_once_it_moves() {
        use vectorcraft_doc::{Appearance, LiveShape, Node, NodeKind};
        let live = LiveShape::Rectangle {
            w: 100.0,
            h: 100.0,
            radii: [0.0; 4],
            kinds: Default::default(),
            xf: vectorcraft_geom::Affine::translate((100.0, 100.0)),
        };
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let id = d.alloc_id();
        let mut n = Node::path(id, live.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(Box::new(live));
        }
        d.insert(Some(d.layers[0].id), 0, n).unwrap();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert!(!t.transforming());
        t.pointer(&cx, &ev(PointerKind::Down, 190.0, 191.0));
        assert!(!t.transforming(), "a press is still a click");
        t.pointer(&cx, &ev(PointerKind::Drag, 175.0, 176.0));
        assert!(t.transforming());
    }

    /// A type-on-path bracket drag is a transform, so the sharp selection box hides while the span moves.
    #[test]
    fn a_bracket_drag_counts_as_transforming_once_it_moves() {
        let (d, id) = crate::testutil::doc_with_path_type();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert!(!t.transforming());
        t.pointer(&cx, &ev(PointerKind::Down, 280.0, 290.0));
        assert!(!t.transforming(), "a press is still a click");
        t.pointer(&cx, &ev(PointerKind::Drag, 320.0, 290.0));
        assert!(t.transforming());
    }

    /// #812: with Smart Guides on, hovering near an object's centre or an anchor labels it, and a
    /// press there grabs the object by that point.
    #[test]
    fn hovering_labels_the_centre_and_anchors_and_a_press_grabs_by_them() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let label = |t: &SelectionTool| {
            t.overlays(&cx).iter().find_map(|o| match o {
                Overlay::Label { p, text, .. } => Some((*p, text.clone())),
                _ => None,
            })
        };
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Move, 152.0, 151.0));
        assert_eq!(label(&t), Some((Point::new(150.0, 150.0), "center".to_string())));
        t.pointer(&cx, &ev(PointerKind::Move, 102.0, 101.0));
        assert_eq!(label(&t), Some((Point::new(100.0, 100.0), "anchor".to_string())));
        t.pointer(&cx, &ev(PointerKind::Move, 130.0, 170.0));
        assert_eq!(label(&t), None, "away from both");
        // Grabbed by its centre, the object's centre follows the pointer.
        t.pointer(&cx, &ev(PointerKind::Move, 152.0, 151.0));
        t.pointer(&cx, &ev(PointerKind::Down, 152.0, 151.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 250.0, 250.0));
        assert!(a.iter().any(|a| matches!(a, Action::Preview(_, v) if v["matrix"][4] == 100.0 && v["matrix"][5] == 100.0)), "{a:?}");
        // With Smart Guides off, nothing is labelled.
        let off = ToolContext { smart_guides: false, ..cx };
        let mut t = SelectionTool::default();
        t.pointer(&off, &ev(PointerKind::Move, 152.0, 151.0));
        assert!(!t.overlays(&off).iter().any(|o| matches!(o, Overlay::Label { .. })));
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

    /// A Cmd/Ctrl+Shift double-click releases the ruler guide or the guide path under the pointer;
    /// without both keys, or while guides are hidden or locked, nothing is released.
    #[test]
    fn a_cmd_shift_double_click_releases_the_guide_under_the_pointer() {
        let (mut d, id) = doc_with_rect();
        d.guides.push(vectorcraft_doc::Guide::new(false, 300.0));
        let Some(vectorcraft_doc::NodeKind::Path { guide, .. }) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("the rectangle is a path") };
        *guide = true;
        let s = Selection::default();
        let p = paint();
        let both = Mods { cmd: true, shift: true, ..Mods::default() };
        let dbl = |c: &ToolContext, x: f64, y: f64, m: Mods| SelectionTool::default().pointer(c, &ev(PointerKind::DoubleClick, x, y).with_mods(m));
        let releases = |a: &[Action]| a.iter().any(|a| matches!(a, Action::Exec(c, _) if c == "view.guides.release"));
        let c = cx(&d, &s, &p);
        assert_eq!(dbl(&c, 350.0, 301.0, both), vec![Action::Exec("view.guides.release".into(), json!({"index": 0}))]);
        assert_eq!(dbl(&c, 200.0, 150.0, both), vec![Action::Exec("view.guides.release".into(), json!({"ids": [id.0]}))]);
        assert!(dbl(&c, 350.0, 301.0, Mods::default()).is_empty(), "a plain double-click on a ruler guide");
        for m in [Mods { cmd: true, ..Mods::default() }, Mods { shift: true, ..Mods::default() }, Mods::default()] {
            assert!(!releases(&dbl(&c, 200.0, 150.0, m)), "{m:?}");
        }
        let locked = ToolContext { guides: false, ..cx(&d, &s, &p) };
        assert!(!releases(&dbl(&locked, 350.0, 301.0, both)));
        assert!(!releases(&dbl(&locked, 200.0, 150.0, both)));
        // A double-click on art that is not a guide does not release it.
        let (plain, _) = doc_with_rect();
        assert!(!releases(&dbl(&cx(&plain, &s, &p), 200.0, 150.0, both)));
    }

    /// Just outside each corner of the box the rotate cursor bows toward that corner, its arc
    /// opening toward the box: at the top right it opens to the bottom left.
    #[test]
    fn the_rotate_cursor_wraps_the_corner_it_is_at() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let t = SelectionTool::default();
        // The box runs from (100, 100) to (200, 200); 10 pt out from each corner, diagonally.
        for ((x, y), octant) in [((93.0, 93.0), 7), ((207.0, 93.0), 1), ((207.0, 207.0), 3), ((93.0, 207.0), 5)] {
            assert_eq!(t.cursor(&cx, Point::new(x, y), Mods::default()), Cursor::RotateToward(octant), "at ({x}, {y})");
        }
        assert_eq!(Cursor::rotate_about(Point::new(1.0, 1.0), Point::new(1.0, 1.0)), Cursor::Rotate, "on the centre");
        assert_eq!(Cursor::rotate_about(Point::ORIGIN, Point::new(0.0, -5.0)), Cursor::RotateToward(0), "straight up");
    }

    #[test]
    fn turned_cursors_step_with_the_view() {
        assert_eq!(Cursor::ResizeH.turned(90.0), Cursor::ResizeV);
        assert_eq!(Cursor::ResizeNeSw.turned(45.0), Cursor::ResizeH);
        assert_eq!(Cursor::ResizeH.turned(45.0), Cursor::ResizeNwSe);
        assert_eq!(Cursor::ResizeNwSe.turned(-135.0), Cursor::ResizeV);
        assert_eq!(Cursor::RotateToward(0).turned(90.0), Cursor::RotateToward(2));
        assert_eq!(Cursor::RotateToward(1).turned(-45.0), Cursor::RotateToward(0));
        assert_eq!(Cursor::RotateToward(7).turned(180.0), Cursor::RotateToward(3));
        assert_eq!(Cursor::ResizeV.turned(20.0), Cursor::ResizeV, "under 22.5° it stays");
        assert_eq!(Cursor::Pen.turned(90.0), Cursor::Pen);
        assert_eq!(Cursor::Rotate.turned(90.0), Cursor::Rotate);
        assert_eq!(Cursor::ResizeH.turned(f64::NAN), Cursor::ResizeH);
        // `rotate_about_in_view` turns the pointer's direction by the view before it rounds: a
        // pointer 15° clockwise from straight up, in a view turned 15°, shows 30° from straight up
        // on screen, which rounds to the top right. `turned` rounds the 15° view to 0°.
        let (sn, cs) = 15f64.to_radians().sin_cos();
        let q = Point::new(sn, -cs);
        assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, q, 15.0), Cursor::RotateToward(1));
        assert_eq!(Cursor::rotate_about(Point::ORIGIN, q).turned(15.0), Cursor::RotateToward(0));
        assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, q, 0.0), Cursor::rotate_about(Point::ORIGIN, q));
        assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, Point::new(0.0, -5.0), 90.0), Cursor::RotateToward(2));
        assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, Point::new(0.0, -5.0), f64::NAN), Cursor::RotateToward(0));
        assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, Point::ORIGIN, 90.0), Cursor::Rotate, "on the center");
    }

    /// In a rotated view the resize and rotate cursors turn with the view.
    #[test]
    fn box_cursors_turn_with_the_view() {
        let (mut d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        d.guides.push(vectorcraft_doc::Guide::new(false, 300.0));
        let mut c = cx(&d, &s, &p);
        c.screen = turned_view(90.0);
        assert!((c.view_angle() - 90.0).abs() < 1e-9);
        let t = SelectionTool::default();
        // The box runs from (100, 100) to (200, 200). Its right side shows at the bottom, and its
        // top shows on the right.
        assert_eq!(t.cursor(&c, Point::new(200.0, 150.0), Mods::default()), Cursor::ResizeV, "right-middle");
        assert_eq!(t.cursor(&c, Point::new(150.0, 100.0), Mods::default()), Cursor::ResizeH, "top-middle");
        assert_eq!(t.cursor(&c, Point::new(207.0, 93.0), Mods::default()), Cursor::RotateToward(3), "out from the top right");
        // While a rotate drags, the cursor turns with the view too: straight up the page shows to
        // the right.
        let mut r = SelectionTool::default();
        assert_eq!(r.pointer(&c, &ev(PointerKind::Down, 207.0, 93.0)), vec![Action::Begin("Rotate".into())]);
        assert_eq!(r.cursor(&c, Point::new(150.0, 80.0), Mods::default()), Cursor::RotateToward(2), "rotating");
        // A horizontal ruler guide shows upright and moves left and right on screen.
        assert_eq!(t.cursor(&c, Point::new(400.0, 300.0), Mods::default()), Cursor::ResizeH, "guide");
        // A box turned 20° in a view turned 25° shows 5° off level: the right handle shows the
        // left-right arrow.
        let (center, a) = (Point::new(150.0, 150.0), 20f64.to_radians());
        let turn = Affine::translate(center.to_vec2()) * Affine::rotate(-a) * Affine::translate(-center.to_vec2());
        d.node_mut(id).unwrap().transform(turn, false);
        let mut c = cx(&d, &s, &p);
        c.screen = turned_view(25.0);
        let right = Point::new(150.0 + 50.0 * a.cos(), 150.0 - 50.0 * a.sin());
        assert_eq!(t.cursor(&c, right, Mods::default()), Cursor::ResizeH, "20° box, 25° view");
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

    /// #1534: with Smart Guides on (the default), the point a selection is dragged by still snaps
    /// to a ruler guide within the Snap to Point distance; not with Snap to Point off.
    #[test]
    fn a_move_snaps_the_grabbed_point_to_a_ruler_guide_with_smart_guides_on() {
        let (mut d, id) = doc_with_rect();
        d.guides = vec![vectorcraft_doc::Guide::new(true, 173.4)];
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let dx = |c: &ToolContext, to: f64| {
            let mut t = SelectionTool::default();
            t.pointer(c, &ev(PointerKind::Down, 150.0, 150.0));
            let a = t.pointer(c, &ev(PointerKind::Drag, to, 150.0));
            let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
            v["matrix"][4].as_f64().unwrap()
        };
        let c = cx(&d, &s, &p);
        assert!(c.smart_guides && c.snap_to_point);
        assert!((dx(&c, 172.0) - 23.4).abs() < 1e-9, "onto the guide");
        assert_eq!(dx(&c, 165.0), 15.0, "out of reach");
        assert_eq!(dx(&ToolContext { snap_to_point: false, ..cx(&d, &s, &p) }, 172.0), 22.0);
    }

    /// Alt pressed during a move makes a copy, which snaps to the original it leaves behind (as
    /// gathered before the previews moved it); released, the move doesn't.
    #[test]
    fn an_alt_copy_snaps_to_the_original_left_behind() {
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, id) = doc_with_rect();
        // An artboard whose edges and centre are out of reach.
        d.artboards[0].rect = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut t = SelectionTool::default();
        let dx = |a: &[Action]| {
            let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
            v["matrix"][4].as_f64().unwrap()
        };
        t.pointer(&cx(&d, &s, &p), &ev(PointerKind::Down, 150.0, 150.0));
        assert_eq!(dx(&t.pointer(&cx(&d, &s, &p), &ev(PointerKind::Drag, 200.0, 150.0))), 50.0);
        // The previews moved the rectangle: the tool sees it 50 pt along.
        let mut moved = d.clone();
        let l = moved.layers[0].id;
        moved.remove(id).unwrap();
        let r = vectorcraft_geom::shapes::rectangle(Rect::new(150.0, 100.0, 250.0, 200.0));
        moved.insert(Some(l), 0, Node::path(id, r, Appearance::default_art())).unwrap();
        let c = cx(&moved, &s, &p);
        let alt = Mods { alt: true, ..Mods::default() };
        // The copy's left edge, 3 pt past the original's right edge (x = 200), lands on it.
        assert_eq!(dx(&t.pointer(&c, &ev(PointerKind::Drag, 253.0, 150.0).with_mods(alt))), 100.0);
        // Without Alt nothing stays behind there.
        assert_eq!(dx(&t.pointer(&c, &ev(PointerKind::Drag, 253.0, 150.0))), 103.0);
    }

    /// A move shows the filled arrow, hollow while it snaps (here B's left side onto A's right
    /// one); Alt, which leaves a copy behind, the double arrowhead.
    #[test]
    fn a_move_shows_the_filled_arrow_hollow_on_a_snap_and_alt_the_copy_cursor() {
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, _) = doc_with_rect();
        d.artboards[0].rect = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let l = d.layers[0].id;
        let b = d.alloc_id();
        let sq = vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 300.0, 400.0, 400.0));
        d.insert(Some(l), 1, Node::path(b, sq, Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.add(b);
        let p = paint();
        let c = cx(&d, &s, &p);
        let none = Mods::default();
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 350.0, 350.0));
        t.pointer(&c, &ev(PointerKind::Drag, 377.0, 371.0));
        assert_eq!(t.cursor(&c, Point::new(377.0, 371.0), none), Cursor::Arrow);
        t.pointer(&c, &ev(PointerKind::Drag, 253.0, 371.0));
        assert_eq!(t.cursor(&c, Point::new(253.0, 371.0), none), Cursor::ArrowHollow, "lined up with A");
        let alt = Mods { alt: true, ..none };
        t.pointer(&c, &ev(PointerKind::Drag, 377.0, 371.0).with_mods(alt));
        assert_eq!(t.cursor(&c, Point::new(377.0, 371.0), alt), Cursor::SelectionCopy);
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

    /// The selection Shift-dragged from (150, 150) to `to`: the move it previews, and its guide
    /// lines.
    fn shift_moved(cx: &ToolContext, to: (f64, f64)) -> (Vec2, Vec<(Point, Point)>) {
        let shift = |kind, x, y| ev(kind, x, y).with_mods(Mods { shift: true, ..Mods::default() });
        let mut t = SelectionTool::default();
        t.pointer(cx, &shift(PointerKind::Down, 150.0, 150.0));
        let a = t.pointer(cx, &shift(PointerKind::Drag, to.0, to.1));
        let Some(Action::Preview(_, v)) = a.get(1) else { panic!("{a:?}") };
        let lines = t.overlays(cx).into_iter().filter_map(|o| if let Overlay::Line { a, b, .. } = o { Some((a, b)) } else { None }).collect();
        (Vec2::new(v["matrix"][4].as_f64().unwrap(), v["matrix"][5].as_f64().unwrap()), lines)
    }

    /// `d` is `want` within 1e-9 on each axis.
    fn moved_by(d: Vec2, want: (f64, f64)) -> bool {
        (d.x - want.0).abs() < 1e-9 && (d.y - want.1).abs() < 1e-9
    }

    /// A Shift-constrained move snapped to the grid stays on its line: a level or upright one lands
    /// on the grid along its own axis only, and a diagonal one slides along the diagonal until an
    /// edge or the center is on a gridline on the axis that needs the shorter way there.
    #[test]
    fn a_shift_move_snapped_to_the_grid_stays_on_its_line() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        // Level by 13.04: the center (163.04) lands on the gridline 162, and the top stays at 100,
        // between the gridlines 99 and 108.
        let (m, _) = shift_moved(&cx, (163.0, 151.0));
        assert!(moved_by(m, (12.0, 0.0)), "{m:?}");
        let (m, _) = shift_moved(&cx, (151.0, 163.0));
        assert!(moved_by(m, (0.0, 12.0)), "{m:?}");
        // Up and to the right by 13 on each axis: the center (163, 137) is 1 from the gridline 162
        // and 2 from 135, so the move goes back along the diagonal by 1 on each axis.
        let (m, _) = shift_moved(&cx, (163.0, 137.0));
        assert!(moved_by(m, (12.0, -12.0)), "{m:?}");
    }

    /// A Shift-constrained move with Snap to Pixel on puts the top-left on a whole pixel on the
    /// axis its line runs along, and on a diagonal on the axis that needs the shorter way there.
    #[test]
    fn a_shift_move_snapped_to_pixels_stays_on_its_line() {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let square = vectorcraft_geom::shapes::rectangle(Rect::new(100.3, 100.4, 200.3, 200.4));
        d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, square, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = ToolContext { snap_to_pixel: true, ..cx(&d, &s, &p) };
        // Pressed at (150, 150), the square is grabbed by its center (150.3, 150.4).
        let (m, _) = shift_moved(&cx, (163.0, 151.0));
        assert!(moved_by(m, (12.7, 0.0)), "{m:?}");
        let (m, _) = shift_moved(&cx, (151.0, 163.0));
        assert!(moved_by(m, (0.0, 12.6)), "{m:?}");
        // By 13.16 on each axis, the top-left (113.46, 113.56) is 0.44 from a whole pixel on y and
        // 0.46 on x.
        let (m, _) = shift_moved(&cx, (163.0, 164.0));
        assert!(moved_by(m, (13.6, 13.6)), "{m:?}");
    }

    /// A Shift-constrained move that Smart Guides line up with another object stays on its line:
    /// a level or upright one lines up along its own axis only, with the guide of that axis, and
    /// a diagonal one slides along the diagonal to the target that needs the shorter way.
    #[test]
    fn a_shift_move_lined_up_by_smart_guides_stays_on_its_line() {
        let with = |r: Rect| {
            let (mut d, id) = doc_with_rect();
            let l = d.layers[0].id;
            let other = d.alloc_id();
            d.insert(
                Some(l),
                1,
                vectorcraft_doc::Node::path(other, vectorcraft_geom::shapes::rectangle(r), vectorcraft_doc::Appearance::default_art()),
            )
            .unwrap();
            (d, id)
        };
        let p = paint();
        // Level by 100.02: the right edge lines up with the other square's left (300). That
        // square's top (203), 3 below the selection's bottom and within reach, does not pull it
        // down.
        let (d, id) = with(Rect::new(300.0, 203.0, 350.0, 253.0));
        let mut s = Selection::default();
        s.add(id);
        let (m, lines) = shift_moved(&cx(&d, &s, &p), (250.0, 152.0));
        assert!(moved_by(m, (100.0, 0.0)), "{m:?}");
        assert!(!lines.is_empty() && lines.iter().all(|(a, b)| a.x == b.x), "only upright guides: {lines:?}");
        let (d, id) = with(Rect::new(203.0, 300.0, 253.0, 350.0));
        let mut s = Selection::default();
        s.add(id);
        let (m, lines) = shift_moved(&cx(&d, &s, &p), (152.0, 250.0));
        assert!(moved_by(m, (0.0, 100.0)), "{m:?}");
        assert!(!lines.is_empty() && lines.iter().all(|(a, b)| a.y == b.y), "only level guides: {lines:?}");
        // By 110.5 on each axis: the right edge (310.5) is 0.5 from the other square's left and the
        // bottom 3 from its top, so the move goes on by 0.5 on each axis.
        let (d, id) = with(Rect::new(311.0, 313.5, 361.0, 363.5));
        let mut s = Selection::default();
        s.add(id);
        let (m, lines) = shift_moved(&cx(&d, &s, &p), (260.5, 260.5));
        assert!(moved_by(m, (111.0, 111.0)), "{m:?}");
        assert!(matches!(lines[..], [(a, b)] if a.x == 311.0 && b.x == 311.0), "{lines:?}");
    }

    /// A level Shift-constrained move that Spacing Guides space evenly keeps its line: the gap to
    /// the next square becomes 60, the gap between the other two, and their tops (102), 2 below
    /// the selection's, do not pull it down.
    #[test]
    fn a_shift_move_spaced_by_smart_guides_stays_on_its_line() {
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        for x in [330.0, 440.0] {
            let other = d.alloc_id();
            let sq = vectorcraft_geom::shapes::rectangle(Rect::new(x, 102.0, x + 50.0, 202.0));
            d.insert(Some(l), 1, vectorcraft_doc::Node::path(other, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
        }
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let (m, lines) = shift_moved(&cx(&d, &s, &p), (221.0, 151.0));
        assert!(moved_by(m, (70.0, 0.0)), "{m:?}");
        assert!(!lines.is_empty(), "the equal gaps are marked");
    }

    /// A Shift-constrained move that snaps by its grabbed point stays on its line: with Smart
    /// Guides off and Snap to Point on, it stops where the line passes nearest an anchor, or
    /// where it crosses a ruler guide; with Smart Guides on, Snap to Point still slides it along
    /// the line onto a ruler guide.
    #[test]
    fn a_shift_move_snapped_by_its_grabbed_point_stays_on_its_line() {
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        let other = d.alloc_id();
        let sq = vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 151.5, 350.0, 201.5));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(other, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
        d.guides = vec![vectorcraft_doc::Guide::new(true, 420.5)];
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let off = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
        // The grabbed point (299, 150) is 1.8 from the anchor (300, 151.5): it stops at (300, 150),
        // the point of the line nearest the anchor.
        let (m, _) = shift_moved(&off, (299.0, 151.0));
        assert!(moved_by(m, (150.0, 0.0)), "{m:?}");
        // On the diagonal, 1 short of the ruler guide at 420.5: it goes on to it on both axes.
        let (m, _) = shift_moved(&off, (419.5, 419.5));
        assert!(moved_by(m, (270.5, 270.5)), "{m:?}");
        // Smart Guides on, the selection alone with a ruler guide 1.5 below the grabbed point.
        let (mut d, id) = doc_with_rect();
        d.guides = vec![vectorcraft_doc::Guide::new(false, 330.5)];
        let mut s = Selection::default();
        s.add(id);
        let (m, _) = shift_moved(&cx(&d, &s, &p), (329.0, 329.0));
        assert!(moved_by(m, (180.5, 180.5)), "{m:?}");
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
    /// the key again lets it go. A drag moves them all, a lone object has no key unless ruler
    /// guides are selected with it.
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
        // One object selected: a click on it does nothing, unless ruler guides are selected
        // with it for it to be the key of (#2143).
        let mut one = Selection::default();
        one.add(rect);
        assert!(click(&one, 150.0, 150.0).is_empty());
        one.guides = vec![0];
        assert_eq!(click(&one, 150.0, 150.0), vec![Action::Exec("select.key".into(), json!({"id": rect.0}))]);
    }

    /// In isolation mode a click makes only a selected top-level object the key, as `select.key`
    /// takes no other: a click on a sublayer's art inside an isolated layer picks the sublayer,
    /// a click inside an isolated group that is also selected picks an object inside it, and a
    /// click inside an isolated compound path picks a member.
    #[test]
    fn a_click_makes_only_a_selected_top_level_object_the_key() {
        use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeKind};
        use vectorcraft_geom::shapes;
        let p = paint();
        let click = |d: &Document, s: &Selection, isolation: NodeId| {
            let c = ToolContext { isolation: Some(isolation), ..cx(d, s, &p) };
            let mut t = SelectionTool::default();
            assert!(t.pointer(&c, &ev(PointerKind::Down, 150.0, 150.0)).is_empty());
            t.pointer(&c, &ev(PointerKind::Up, 150.0, 150.0))
        };
        let path = |id: NodeId, r: Rect| Node::path(id, shapes::rectangle(r), Appearance::default_art());
        let (under, beside) = (Rect::new(100.0, 100.0, 200.0, 200.0), Rect::new(300.0, 300.0, 340.0, 340.0));
        // An isolated layer whose art under the click is in a sublayer.
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (sub, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        d.insert(Some(l), 0, Node::layer(sub, "Sub", LayerColor::Preset(1))).unwrap();
        d.insert(Some(sub), 0, path(a, under)).unwrap();
        d.insert(Some(l), 1, path(b, beside)).unwrap();
        let mut s = Selection::default();
        s.add(sub);
        s.add(b);
        assert!(click(&d, &s, l).is_empty(), "a sublayer is not a key object");
        // An isolated group that is also selected.
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (g, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        d.insert(Some(l), 0, Node::group(g, vec![])).unwrap();
        d.insert(Some(g), 0, path(a, under)).unwrap();
        d.insert(Some(g), 1, path(b, beside)).unwrap();
        let mut s = Selection::default();
        s.add(a);
        s.add(g);
        assert!(click(&d, &s, g).is_empty(), "an object inside the selected group is not a key object");
        s.key = Some(a);
        assert_eq!(click(&d, &s, g), vec![Action::Exec("select.key".into(), json!({}))], "the key again: no key");
        // With the group not selected, the object becomes the key.
        let mut s = Selection::default();
        s.add(a);
        s.add(b);
        assert_eq!(click(&d, &s, g), vec![Action::Exec("select.key".into(), json!({"id": a.0}))]);
        // An isolated compound path. Make Compound Path clears its members' appearance, so these
        // members have none, and the click is on a's outline.
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (c, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let member = |id: NodeId, r: Rect| std::sync::Arc::new(Node::path(id, shapes::rectangle(r), Appearance::default()));
        let children = vec![member(a, Rect::new(150.0, 100.0, 250.0, 200.0)), member(b, beside)];
        let mut compound = Node::new(c, NodeKind::Compound { children, rule: Default::default() });
        compound.appearance = Appearance::default_art().into();
        d.insert(Some(l), 0, compound).unwrap();
        let mut s = Selection::default();
        s.add(a);
        s.add(b);
        assert!(click(&d, &s, c).is_empty(), "a compound-path member is not a key object");
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

    /// E pressed during a marquee drag makes the marquee take only the objects entirely inside it
    /// (a group whole), and E pressed again makes it take what it touches. Only a marquee takes E.
    #[test]
    fn e_during_a_marquee_takes_only_the_objects_inside() {
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, inside) = doc_with_rect();
        let l = d.layers[0].id;
        let rect = |id, r| Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art());
        let (touched, g, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
        d.insert(Some(l), 1, rect(touched, Rect::new(250.0, 100.0, 350.0, 200.0))).unwrap();
        // A group with one member inside the marquee and one outside it.
        d.insert(Some(l), 2, Node::group(g, vec![])).unwrap();
        d.insert(Some(g), 0, rect(a, Rect::new(100.0, 220.0, 120.0, 240.0))).unwrap();
        d.insert(Some(g), 1, rect(b, Rect::new(100.0, 300.0, 120.0, 320.0))).unwrap();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let e = ToolKey::Letter('e');
        let drag = |presses: usize, mods: Mods| {
            let mut t = SelectionTool::default();
            assert!(!t.claims_key(&cx, e), "no marquee yet");
            t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0).with_mods(mods));
            t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 250.0).with_mods(mods));
            for _ in 0..presses {
                assert!(t.claims_key(&cx, e));
                assert!(t.key(&cx, e, Mods::default()).is_empty());
            }
            let out = t.pointer(&cx, &ev(PointerKind::Up, 300.0, 250.0).with_mods(mods));
            assert!(!t.claims_key(&cx, e), "the marquee is over");
            out
        };
        let set = |ids: &[NodeId]| vec![Action::Exec("select.set".into(), json!({ "ids": json_ids(ids) }))];
        assert_eq!(drag(0, Mods::default()), set(&[inside, touched, g]));
        assert_eq!(drag(1, Mods::default()), set(&[inside]));
        assert_eq!(drag(2, Mods::default()), set(&[inside, touched, g]), "E again: what it touches");
        let shift = Mods { shift: true, ..Mods::default() };
        assert_eq!(drag(1, shift), vec![Action::Exec("select.toggle".into(), json!({ "ids": [inside.0] }))]);
        // The next marquee starts out taking what it touches.
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 250.0));
        t.key(&cx, e, Mods::default());
        t.pointer(&cx, &ev(PointerKind::Up, 300.0, 250.0));
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 250.0)), set(&[inside, touched, g]));
        // A press on an object starts a move, which does not take E.
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
        assert!(t.busy() && !t.claims_key(&cx, e));
    }

    /// With E pressed during the drag, a marquee takes only the artboard guides that lie entirely
    /// inside it, and no canvas guide, which runs across the whole canvas.
    #[test]
    fn e_during_a_marquee_takes_only_the_guides_inside() {
        let (mut d, id) = doc_with_rect();
        let board = d.artboards[0].id;
        d.guides.push(vectorcraft_doc::Guide::new(true, 150.0));
        d.guides.push(vectorcraft_doc::Guide { artboard: Some(board), ..vectorcraft_doc::Guide::new(false, 150.0) });
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let marquee = |to: f64| {
            let mut t = SelectionTool::default();
            t.pointer(&cx, &ev(PointerKind::Down, -10.0, -10.0));
            t.pointer(&cx, &ev(PointerKind::Drag, to, to));
            t.key(&cx, ToolKey::Letter('e'), Mods::default());
            t.pointer(&cx, &ev(PointerKind::Up, to, to))
        };
        let guides = |a: &[Action]| a.iter().find_map(|a| matches!(a, Action::Exec(c, _) if c == "guide.select").then(|| a.clone()));
        // The artboard runs from 0 to 500: a marquee to (510, 510) holds its guide.
        let a = marquee(510.0);
        assert_eq!(a.first(), Some(&Action::Exec("select.set".into(), json!({ "ids": [id.0] }))));
        assert_eq!(guides(&a), Some(Action::Exec("guide.select".into(), json!({ "indexes": [1], "toggle": true }))));
        // A marquee to (250, 250) crosses both guides and holds neither.
        assert_eq!(guides(&marquee(250.0)), None);
    }

    #[test]
    fn marquee_selects_ruler_guides() {
        let (mut d, id) = doc_with_rect();
        d.guides.push(vectorcraft_doc::Guide::new(false, 80.0));
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 120.0, 120.0));
        let a = t.pointer(&cx, &ev(PointerKind::Up, 120.0, 120.0));
        assert_eq!(
            a,
            vec![
                Action::Exec("select.set".into(), json!({"ids": [id.0]})),
                Action::Exec("guide.select".into(), json!({"indexes": [0], "toggle": true})),
            ]
        );
    }

    #[test]
    fn marquee_selects_only_guide() {
        let (mut d, _) = doc_with_rect();
        d.guides.push(vectorcraft_doc::Guide::new(true, 20.0));
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 30.0, 30.0));
        let a = t.pointer(&cx, &ev(PointerKind::Up, 30.0, 30.0));
        assert_eq!(
            a,
            vec![Action::Exec("select.set".into(), json!({"ids": []})), Action::Exec("guide.select".into(), json!({"indexes": [0], "toggle": true})),]
        );
    }

    #[test]
    fn marquee_ignores_locked_guides() {
        let (mut d, id) = doc_with_rect();
        d.guides.push(vectorcraft_doc::Guide::new(false, 80.0));
        let s = Selection::default();
        let p = paint();
        let mut cx_locked = cx(&d, &s, &p);
        cx_locked.guides = false;
        let mut t = SelectionTool::default();
        t.pointer(&cx_locked, &ev(PointerKind::Down, 50.0, 50.0));
        t.pointer(&cx_locked, &ev(PointerKind::Drag, 120.0, 120.0));
        let a = t.pointer(&cx_locked, &ev(PointerKind::Up, 120.0, 120.0));
        assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    }

    /// #3036: with Lock Guides on, a guide object neither takes the press nor stops a marquee
    /// from starting over it; with guides unlocked, the press still picks it.
    #[test]
    fn marquee_passes_through_locked_guide_objects() {
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        let g = d.alloc_id();
        let pts = [Point::new(20.0, 20.0), Point::new(400.0, 20.0), Point::new(400.0, 400.0), Point::new(20.0, 400.0)];
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, true));
        let mut guide = vectorcraft_doc::Node::path(g, path, vectorcraft_doc::Appearance::default_art());
        let vectorcraft_doc::NodeKind::Path { guide: flag, .. } = &mut guide.kind else { panic!("a path") };
        *flag = true;
        d.insert(Some(l), 1, guide).unwrap();
        let s = Selection::default();
        let p = paint();
        let locked = ToolContext { guides: false, ..cx(&d, &s, &p) };
        for (x, y) in [(20.0, 60.0), (50.0, 50.0)] {
            let mut t = SelectionTool::default();
            assert!(t.pointer(&locked, &ev(PointerKind::Down, x, y)).is_empty(), "press at ({x}, {y})");
            t.pointer(&locked, &ev(PointerKind::Drag, 250.0, 250.0));
            assert!(matches!(t.state, State::Marquee { .. }), "press at ({x}, {y})");
            // The rectangle is rubber-band selected (the engine leaves locked guides out).
            let a = t.pointer(&locked, &ev(PointerKind::Up, 250.0, 250.0));
            let Some(Action::Exec(c, v)) = a.first() else { panic!("press at ({x}, {y}): {a:?}") };
            assert_eq!(c, "select.set", "press at ({x}, {y})");
            assert!(v["ids"].as_array().unwrap().contains(&json!(id.0)), "press at ({x}, {y}): {a:?}");
        }
        let unlocked = ToolContext { guides: true, ..cx(&d, &s, &p) };
        let mut t = SelectionTool::default();
        assert_eq!(t.pointer(&unlocked, &ev(PointerKind::Down, 20.0, 60.0)), vec![Action::Exec("select.set".into(), json!({"ids": [g.0]}))]);
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

    /// The box the 100–200 rectangle's bounds become under the scale a handle drag previews.
    fn scaled(a: &[Action]) -> Rect {
        let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
        let m: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]]).transform_rect_bbox(Rect::new(100.0, 100.0, 200.0, 200.0))
    }

    fn near(a: Rect, b: Rect) -> bool {
        [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 1e-9)
    }

    /// #2054: with Snap to Grid on, a handle drag lands what the handle moves on the nearest
    /// gridline (every 9 pt by default) while the opposite side stays put; Shift keeps the
    /// proportions, and Alt keeps the center.
    #[test]
    fn handle_drag_lands_what_it_moves_on_the_grid() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let c = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        let drag = |from: (f64, f64), to: (f64, f64), mods: Mods| {
            let mut t = SelectionTool::default();
            assert_eq!(t.pointer(&c, &ev(PointerKind::Down, from.0, from.1).with_mods(mods)), vec![Action::Begin("Scale".into())]);
            scaled(&t.pointer(&c, &ev(PointerKind::Drag, to.0, to.1).with_mods(mods)))
        };
        let none = Mods::default();
        // The right side, dragged to x = 213, lands on the gridline at 216.
        let r = drag((200.0, 150.0), (213.0, 150.0), none);
        assert!(near(r, Rect::new(100.0, 100.0, 216.0, 200.0)), "{r:?}");
        // The bottom-right corner, dragged to (213, 204), lands on (216, 207).
        let r = drag((200.0, 200.0), (213.0, 204.0), none);
        assert!(near(r, Rect::new(100.0, 100.0, 216.0, 207.0)), "{r:?}");
        // Shift: the corner, at (213, 213) in proportion, lands on (216, 216), still square.
        let r = drag((200.0, 200.0), (213.0, 204.0), Mods { shift: true, ..none });
        assert!(near(r, Rect::new(100.0, 100.0, 216.0, 216.0)), "{r:?}");
        // Alt: the right side lands on 216 and the left side mirrors it about the center.
        let r = drag((200.0, 150.0), (213.0, 150.0), Mods { alt: true, ..none });
        assert!(near(r, Rect::new(84.0, 100.0, 216.0, 200.0)), "{r:?}");
    }

    /// #2054: with Snap to Pixel on, what a handle drag moves lands on whole pixels.
    #[test]
    fn handle_drag_lands_what_it_moves_on_whole_pixels() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let c = ToolContext { snap_to_pixel: true, ..cx(&d, &s, &p) };
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0));
        let r = scaled(&t.pointer(&c, &ev(PointerKind::Drag, 212.4, 230.7)));
        assert!(near(r, Rect::new(100.0, 100.0, 212.0, 231.0)), "{r:?}");
    }

    /// #2054: with Smart Guides off, View › Snap to Point lands a dragged handle on another
    /// object's anchor or a ruler guide within its distance (2 px); with Smart Guides on, the
    /// ruler guides still pull it within the Snapping Tolerance (4 px), as a move does (#1534).
    #[test]
    fn handle_drag_snaps_to_anchors_and_ruler_guides_with_snap_to_point() {
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        let other = d.alloc_id();
        let sq = vectorcraft_geom::shapes::rectangle(Rect::new(250.0, 250.0, 300.0, 300.0));
        d.insert(Some(l), 1, Node::path(other, sq, Appearance::default_art())).unwrap();
        d.guides = vec![vectorcraft_doc::Guide::new(true, 230.5)];
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let drag = |c: &ToolContext, from: (f64, f64), to: (f64, f64)| {
            let mut t = SelectionTool::default();
            t.pointer(c, &ev(PointerKind::Down, from.0, from.1));
            scaled(&t.pointer(c, &ev(PointerKind::Drag, to.0, to.1)))
        };
        let off = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
        assert!(off.snap_to_point);
        // The corner, 1.4 px from the other square's corner, lands on it.
        let r = drag(&off, (200.0, 200.0), (249.0, 251.0));
        assert!(near(r, Rect::new(100.0, 100.0, 250.0, 250.0)), "{r:?}");
        // The right side, 1 px from the ruler guide, lands on it.
        let r = drag(&off, (200.0, 150.0), (231.5, 150.0));
        assert!(near(r, Rect::new(100.0, 100.0, 230.5, 200.0)), "{r:?}");
        // Snap to Point off: the side follows the pointer.
        let r = drag(&ToolContext { snap_to_point: false, ..off }, (200.0, 150.0), (231.5, 150.0));
        assert!(near(r, Rect::new(100.0, 100.0, 231.5, 200.0)), "{r:?}");
        // Smart Guides on: the ruler guide, 2.5 px away, pulls the side too.
        let r = drag(&cx(&d, &s, &p), (200.0, 150.0), (233.0, 150.0));
        assert!(near(r, Rect::new(100.0, 100.0, 230.5, 200.0)), "{r:?}");
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

    /// [`doc_with_area_type`] with more text than its frame holds, selected; the rectangle's id and
    /// where the frame's out port is.
    fn overflowing_area_type() -> (vectorcraft_doc::Document, NodeId, NodeId, Point) {
        let (mut d, id) = doc_with_area_type();
        let rect = d.children(Some(d.layers[0].id)).unwrap()[0].id;
        let Some(vectorcraft_doc::NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("not text") };
        let style = vectorcraft_doc::CharStyle { size: 20.0, ..Default::default() };
        t.runs = vec![vectorcraft_doc::TextRun::new("Words that fill more than the frame holds. ".repeat(20), style)];
        // On the frame's right side, above its bottom-right corner (120 × 40 at 100%).
        let port = t.xf * Point::new(120.0, 40.0 - crate::text::PORT_RISE_PX);
        (d, id, rect, port)
    }

    /// #2572: a click on the out port loads the pointer with the text; a click on empty canvas then
    /// threads it into a new frame there, a click on a closed path into the path, a drag into the
    /// rectangle it draws.
    #[test]
    fn a_click_on_the_out_port_loads_the_text_and_the_next_click_or_drag_threads_it() {
        let (d, id, rect, port) = overflowing_area_type();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        let thread = |params: Value| vec![Action::Exec("text.thread.add".into(), params)];
        let load = |t: &mut SelectionTool| {
            assert!(t.pointer(&cx, &ev(PointerKind::Down, port.x, port.y)).is_empty());
            assert!(t.pointer(&cx, &ev(PointerKind::Up, port.x, port.y)).is_empty());
            assert_eq!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText);
        };
        load(&mut t);
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 30.0, 420.0)).is_empty());
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 30.0, 420.0)), thread(json!({"from": id.0, "at": [30.0, 420.0]})));
        assert_ne!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText, "the text is placed");
        load(&mut t);
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 150.0, 150.0)), thread(json!({"from": id.0, "to": rect.0})));
        load(&mut t);
        t.pointer(&cx, &ev(PointerKind::Down, 20.0, 20.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 80.0, 90.0));
        assert!(t.overlays(&cx).contains(&Overlay::Marquee(Rect::new(20.0, 20.0, 80.0, 90.0))), "the new frame's outline");
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 80.0, 90.0)), thread(json!({"from": id.0, "rect": [20.0, 20.0, 60.0, 70.0]})));
        // The bounding box's corner handle below the port still resizes the frame.
        let corner = Point::new(port.x, port.y + crate::text::PORT_RISE_PX);
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, corner.x, corner.y)), vec![Action::Begin("Resize Type Area".into())]);
    }

    /// Escape drops the loaded text, and the next click is an ordinary one; type that fits its
    /// frame shows no out port to click.
    #[test]
    fn escape_drops_the_loaded_text_and_type_that_fits_has_no_port() {
        let (d, id, _, port) = overflowing_area_type();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, port.x, port.y));
        t.pointer(&cx, &ev(PointerKind::Up, port.x, port.y));
        assert!(t.claims_key(&cx, ToolKey::Escape), "Escape is the loaded pointer's");
        assert!(t.key(&cx, ToolKey::Escape, Mods::default()).is_empty());
        assert!(!t.claims_key(&cx, ToolKey::Escape));
        assert_ne!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText);
        t.pointer(&cx, &ev(PointerKind::Down, 30.0, 420.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 30.0, 420.0)), vec![Action::Exec("select.none".into(), json!({}))]);

        let (d, id) = doc_with_area_type();
        let mut s = Selection::default();
        s.set([id]);
        let cx = crate::testutil::cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, port.x, port.y));
        t.pointer(&cx, &ev(PointerKind::Up, port.x, port.y));
        assert_ne!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText);
    }
}
