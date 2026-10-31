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
//! Escape during a drag (a move or an Alt-drag copy, a scale, a rotation, a marquee, a widget or
//! bracket drag) cancels it: the art returns to its state at the press.
//! Selected area type and type on a path show an in port and an out port. A click on the out port
//! loads the pointer with the text ([`Cursor::LoadedText`]): the next click threads it into the
//! closed path, area type or type on a path under the pointer, or into a new frame of the same
//! kind, shape and size at the click on empty canvas, and a drag draws a rectangle for the new
//! frame (`text.thread.add`), which comes after the clicked one in its thread. A click on the in
//! port does the same with the new frame put before it: the story then starts there and flows on
//! into the clicked one. Escape drops the loaded text. A double-click on a port where the frame is
//! threaded splits the thread at that port (`text.thread.break`), as the archived help section on
//! threading describes
//! (<https://web.archive.org/web/20220127090642/https://helpx.adobe.com/illustrator/using/creating-text.html>).
//! With Smart Guides and Invoke Distance Guides on, Alt/Option held over another object shows how
//! far the selection is from it, and over an empty part of an artboard how far it is from the
//! artboard's edges ([`crate::guides::distance_guides`]).

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::{Value, json};
use vectorcraft_doc::hit::{Hit, HitKind, hit_test, marquee, objects_at, selected_hidden_at};
use vectorcraft_doc::{NodeId, OrientedBox};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::bbox::{Handle, hit_handle, in_rotate_zone, move_delta, rotate_for_drag, scale_for_drag};
use crate::corners::{self, CornerDrag, over_widget};
use crate::guides::{PointSnap, Targets, distance_guides, hold_on_path};
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
        /// Where the pointer went down: the drag threshold counts from there, not from `start`,
        /// which may be an anchor or centre the press grabbed a few pixels away (#3716).
        press: Point,
        /// The point that snaps, moving with the pointer: `start`, or where a press on a path's
        /// stroke or beside its edge took hold of the path ([`hold_point`]).
        hold: Point,
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
    /// Drag a grid repeat's row, column or corner handle or a spacing slider.
    GridRepeat(crate::repeat_grid::GridDrag),
    /// Dragging a bracket of type on a path.
    Bracket(BracketDrag),
    /// Loaded with the text of area type or type on a path `from` by a click on its out port, or
    /// on its in port (`before`: the frame it goes into comes before `from` in its thread);
    /// `press`: where the next press went down and where the pointer is now.
    Loaded {
        from: NodeId,
        before: bool,
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
    /// The params of the move's last preview: in Shaper's Construction mode the move ends with
    /// them and `releaseOutside` ([`construction`]).
    last_move: Option<Value>,
    /// Held state used for the most recent bounding-box preview.
    last_backquote: bool,
    /// Pointer position used for the most recent bounding-box preview.
    last_box_pos: Option<Point>,
    /// The move snapped to a point or a guide at its last step: the arrow turns hollow.
    snapped: bool,
    guide: GuideEdit,
    /// Smart Guides: the centre or anchor under the pointer, and its label ([`Self::hover_point`]).
    hover: Option<(Point, &'static str)>,
    /// Distance Guides: what Alt held over it measures the selection to ([`distance_target`]).
    distance: Option<DistanceTo>,
    /// The last hover had Alt held: what it measures depends on what lies under the pointer
    /// ([`Tool::wants_rehover`]).
    alt_hover: bool,
}

/// What Distance Guides measure the selection to.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DistanceTo {
    Object(NodeId),
    /// The artboard at this index.
    Artboard(usize),
}

/// Distance Guides: with Smart Guides and Smart Guides → Invoke Distance Guides on, something
/// selected and Alt held (`m`), the object under `p` unless it is selected, or over no object the
/// artboard `p` is on.
fn distance_target(cx: &ToolContext, p: Point, m: Mods) -> Option<DistanceTo> {
    if !(cx.smart_guides && cx.distance_guides && m.alt) || cx.selection.objects.is_empty() {
        return None;
    }
    match hit_test(cx.doc, p, cx.selection_hit_options()).map(|h| h.top_object(cx.isolation)) {
        Some(id) => (!cx.selection.contains(id)).then_some(DistanceTo::Object(id)),
        None => cx.doc.artboard_at(p).map(DistanceTo::Artboard),
    }
}

/// The Distance Guides from the selection to `to` ([`distance_guides`]); none once either is gone.
fn distance_overlays(cx: &ToolContext, to: DistanceTo) -> Vec<Overlay> {
    let Some(from) = selection_bounds(cx) else { return vec![] };
    match to {
        DistanceTo::Object(id) if !cx.selection.contains(id) => object_bounds(cx, id).map(|r| distance_guides(cx, from, r, false)),
        DistanceTo::Object(_) => None,
        DistanceTo::Artboard(i) => cx.doc.artboards.get(i).map(|a| distance_guides(cx, from, a.rect, true)),
    }
    .unwrap_or_default()
}

/// Snapping for the selection moved as a whole, its targets gathered when the move begins: View →
/// Snap to Grid lands its edges on the grid (#740), taking over from the rest; otherwise Smart
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

    /// Snap to Grid and Snap to Pixel alone, for a move whose box is `bounds`: the grid lands an
    /// edge of the box on a gridline, and pixels put its top-left on whole pixels.
    pub(crate) fn grid_only(bounds: Option<Rect>) -> Self {
        Self { bounds, targets: None, points: None, copy_targets: None, copy_points: None, rulers: None }
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
/// two edges is nearest a gridline, however far that is. The center does not snap (#1816).
fn grid_pull(r: Rect, step: f64) -> Vec2 {
    let pull = |vs: [f64; 2]| {
        vs.iter().map(|v| vectorcraft_geom::snap::snap_to_grid(*v, step, 0.0) - v).min_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(0.0)
    };
    Vec2::new(pull([r.x0, r.x1]), pull([r.y0, r.y1]))
}

pub fn matrix_json(a: Affine) -> Value {
    let c = a.as_coeffs();
    json!([c[0], c[1], c[2], c[3], c[4], c[5]])
}

/// The bounds the selection tools measured for one state of an open document: [`selection_bounds`],
/// [`selection_box`] and the objects' bounds that Smart Guides find centers in while hovering
/// ([`object_bounds`]). With Use Preview Bounds on, each measurement evaluates the geometry effects
/// of the objects (Offset Path, Pathfinder, Outline Object, blends), and the canvas asks for the
/// cursor on every hover frame. The session keeps one memo ([`ToolContext::preview_memo`]), as the
/// UI keeps one for `VectorcraftApp::selection_box`. The memo holds the measurements for one key
/// (the document's uid and revision, the selected objects and Use Preview Bounds), and a new key
/// replaces them.
#[derive(Default)]
pub struct PreviewMemo(RefCell<MemoState>);

#[derive(Default)]
struct MemoState {
    entry: Option<Measured>,
    /// How many measurements the memo has made.
    measurements: u64,
}

/// The measurements for one key; a slot holds `None` until it is measured.
struct Measured {
    revision: (u64, u64),
    objects: Vec<NodeId>,
    preview: bool,
    bounds: Option<Option<Rect>>,
    oriented: Option<Option<OrientedBox>>,
    by_object: HashMap<NodeId, Option<Option<Rect>>>,
}

impl Measured {
    fn is_for(&self, cx: &ToolContext) -> bool {
        self.revision == cx.revision && self.preview == cx.preview_bounds && self.objects == cx.selection.objects
    }
}

impl PreviewMemo {
    /// How many measurements the memo has made: each one it could not answer from the measurements
    /// for the document as it is.
    pub fn measurements(&self) -> u64 {
        self.0.try_borrow().map_or(0, |st| st.measurements)
    }

    /// The measurements for the state `cx` shows, after the ones for any other state are dropped.
    fn current<'m>(st: &'m mut MemoState, cx: &ToolContext) -> &'m mut Measured {
        let fresh = || Measured {
            revision: cx.revision,
            objects: cx.selection.objects.clone(),
            preview: cx.preview_bounds,
            bounds: None,
            oriented: None,
            by_object: HashMap::new(),
        };
        if !st.entry.as_ref().is_some_and(|m| m.is_for(cx)) {
            st.entry = Some(fresh());
        }
        st.entry.get_or_insert_with(fresh)
    }

    /// The value `slot` holds for the state `cx` shows, else `measure`'s, which `slot` then holds.
    fn remember<T: Clone>(&self, cx: &ToolContext, slot: impl Fn(&mut Measured) -> &mut Option<T>, measure: impl FnOnce() -> T) -> T {
        if let Ok(mut st) = self.0.try_borrow_mut()
            && let Some(v) = slot(Self::current(&mut st, cx)).clone()
        {
            return v;
        }
        let v = measure();
        if let Ok(mut st) = self.0.try_borrow_mut() {
            st.measurements += 1;
            *slot(Self::current(&mut st, cx)) = Some(v.clone());
        }
        v
    }
}

/// `measure`'s value, kept in the session's [`PreviewMemo`] in the slot `slot` picks; measured
/// every time without a memo.
fn remembered<T: Clone>(cx: &ToolContext, slot: impl Fn(&mut Measured) -> &mut Option<T>, measure: impl FnOnce() -> T) -> T {
    match cx.preview_memo {
        Some(memo) => memo.remember(cx, slot, measure),
        None => measure(),
    }
}

/// Selection bounds used for the bounding box: visual bounds as drawn with Use Preview Bounds
/// ([`vectorcraft_effects::preview`]), else geometric bounds.
pub fn selection_bounds(cx: &ToolContext) -> Option<Rect> {
    remembered(cx, |m| &mut m.bounds, || vectorcraft_effects::preview::bounds_of(cx.doc, &cx.selection.objects, cx.preview_bounds))
}

/// [`selection_bounds`] square to the selection's own angle: the bounding box the Selection tool
/// shows and drags (rotated after a rotation).
pub fn selection_box(cx: &ToolContext) -> Option<OrientedBox> {
    remembered(cx, |m| &mut m.oriented, || vectorcraft_effects::preview::oriented_bounds(cx.doc, &cx.selection.objects, cx.preview_bounds))
}

/// The bounds of object `id`, measured as [`selection_bounds`] measures the selection.
fn object_bounds(cx: &ToolContext, id: NodeId) -> Option<Rect> {
    remembered(cx, |m| m.by_object.entry(id).or_insert(None), || vectorcraft_effects::preview::bounds_of(cx.doc, &[id], cx.preview_bounds))
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

/// Where a press at `p` holds the object it hit (`h`) for a move: the nearest point of the path
/// hit, when the press is on its stroke or outline or within [`HOVER_REACH`] of it, else `p`
/// ([`hold_on_path`]). That point snaps, so the path itself lands on a ruler guide (#3701).
pub(crate) fn hold_point(cx: &ToolContext, h: &Hit, p: Point) -> Point {
    let reach = if matches!(h.kind, HitKind::Stroke | HitKind::Outline) { f64::INFINITY } else { cx.tol(HOVER_REACH) };
    cx.doc.node(h.leaf).and_then(|n| n.path_data()).and_then(|pd| hold_on_path(pd, p, reach)).unwrap_or(p)
}

/// While an opacity mask or a pattern is edited, a Shaper double-click selects a Shaper Group's
/// original shape without isolating the group's original shapes. A press at `p` on a selected
/// original shape holds it for a move: the hit inside its Shaper Group's original shapes, and that
/// container. Each selected original shape's Shaper Group is checked, front to back, and an object
/// in front of that group at `p` keeps the press. None otherwise.
fn selected_source_hit(cx: &ToolContext, p: Point) -> Option<(Hit, NodeId)> {
    if cx.doc.mask_edit.is_none() && cx.doc.pattern_edit.is_none() {
        return None;
    }
    let is_sources = |g: &NodeId| cx.doc.node(*g).is_some_and(|n| n.name.as_deref() == Some(vectorcraft_doc::shaper::SOURCES));
    let mut containers = cx.doc.paint_order(cx.selection.objects.iter().filter_map(|id| cx.doc.parent_of(*id)).filter(is_sources));
    containers.dedup();
    let front = hit_test(cx.doc, p, cx.selection_hit_options());
    containers.into_iter().rev().find_map(|sources| {
        let group = cx.doc.parent_of(sources)?;
        if let Some(f) = &front
            && f.leaf != group
            && !f.ancestry.contains(&group)
            && cx.doc.paint_order([group, f.leaf]).last() == Some(&f.leaf)
        {
            return None;
        }
        let h = hit_test(cx.doc, p, vectorcraft_doc::hit::HitOptions { scope: Some(sources), ..cx.selection_hit_options() })?;
        cx.selection.contains(h.top_object(Some(sources))).then_some((h, sources))
    })
}

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
        let under = hit_test(cx.doc, p, cx.selection_hit_options()).map(|h| h.top_object(cx.isolation));
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
                let c = object_bounds(cx, id)?.center();
                (c.distance(p) <= reach).then_some((c, "center"))
            })
        })
    }

    /// The pointer loaded with the text of area type or type on a path `from` ([`State::Loaded`]):
    /// a press starts placing it, and its release threads it (`text.thread.add`), into a frame put
    /// `before` `from` or after it. None when `from` can no longer thread: the press is an
    /// ordinary one.
    fn loaded(&mut self, cx: &ToolContext, ev: &PointerEvent, from: NodeId, before: bool, press: Option<(Point, Point)>) -> Option<Vec<Action>> {
        let p = ev.pos;
        match (ev.kind, press) {
            // The second press of a double-click on the port that loaded the text: the text is
            // put down, and the double-click breaks the thread there.
            (PointerKind::Down, _)
                if crate::text::port_at(cx, p) == Some((from, if before { crate::text::Port::In } else { crate::text::Port::Out })) =>
            {
                self.state = State::Idle;
                return Some(vec![]);
            }
            (PointerKind::Down, _) if !cx.doc.node(from).is_some_and(is_threadable) => {
                self.state = State::Idle;
                return None;
            }
            (PointerKind::Down, _) => self.state = State::Loaded { from, before, press: Some((p, p)) },
            (PointerKind::Drag, Some((start, _))) => self.state = State::Loaded { from, before, press: Some((start, p)) },
            (PointerKind::Up, Some((start, _))) => {
                self.state = State::Idle;
                let mut params = thread_params(cx, from, start, p);
                if before {
                    params["before"] = json!(true);
                }
                return Some(vec![Action::Exec("text.thread.add".into(), params)]);
            }
            _ => {}
        }
        Some(vec![])
    }
}

/// Where the text of `from` goes for a press at `start` released at `end`: a dragged rectangle,
/// else the closed path, area type or type on a path pressed on, else a new frame with its
/// top-left at `start`.
fn thread_params(cx: &ToolContext, from: NodeId, start: Point, end: Point) -> Value {
    let r = Rect::from_points(start, end);
    if r.width() > cx.tol(6.0) && r.height() > cx.tol(6.0) {
        return json!({ "from": from.0, "rect": [r.x0, r.y0, r.width(), r.height()] });
    }
    let frame = |n: &vectorcraft_doc::Node| {
        is_threadable(n) || matches!(&n.kind, vectorcraft_doc::NodeKind::Path { path, .. } if path.subpaths.first().is_some_and(|s| s.closed))
    };
    // The art the text flows into: what is drawn, not a clipping path's outline.
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
    /// objects inside it. Escape during a drag (a move or an Alt-drag copy, a bounding-box scale or
    /// rotation, a marquee, a corner widget, a pie widget, the brackets of type on a path, a mirror
    /// axis, a grid repeat's handles, a ruler guide) cancels it: what it changed goes back, no copy
    /// is left, and the tool waits for the next press. With the pointer loaded with text, Escape
    /// drops the text.
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
        if key == ToolKey::Escape
            && matches!(self.state, State::GridRepeat(_))
            && let State::GridRepeat(d) = std::mem::take(&mut self.state)
        {
            return d.cancel();
        }
        if key == ToolKey::Escape && matches!(self.state, State::Loaded { .. }) {
            self.state = State::Idle;
            return vec![];
        }
        if key == ToolKey::Escape && !matches!(self.state, State::Idle) {
            self.state = State::Idle;
            self.measure = None;
            self.guides.clear();
            (self.targets, self.moving, self.snapped) = (None, None, false);
            return vec![Action::Cancel];
        }
        if key == ToolKey::Escape { self.guide.cancel() } else { vec![] }
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        match self.state {
            State::Marquee { .. } => key == ToolKey::Letter('e'),
            State::Loaded { .. } | State::Mirror(_) | State::GridRepeat(_) => key == ToolKey::Escape,
            _ => false,
        }
    }

    /// The pointer hovers nothing for the tool: its Smart Guides label and its Distance Guides go.
    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
        self.distance = None;
        self.alt_hover = false;
    }

    fn wants_rehover(&self) -> bool {
        self.alt_hover
    }

    fn transforming(&self) -> bool {
        match &self.state {
            State::Moving { began: true, .. } | State::Scaling { .. } | State::Rotating { .. } => true,
            State::Corner(c) => c.adjusting(),
            State::Pie(d) => d.adjusting(),
            State::Bracket(b) => b.adjusting(),
            State::Mirror(d) => d.adjusting(),
            State::GridRepeat(d) => d.adjusting(),
            _ => false,
        }
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let hovering = ev.kind == PointerKind::Move && matches!(self.state, State::Idle);
        self.alt_hover = hovering && ev.mods.alt;
        self.distance = hovering.then(|| distance_target(cx, ev.pos, ev.mods)).flatten();
        if let State::Loaded { from, before, press } = self.state
            && let Some(out) = self.loaded(cx, ev, from, before, press)
        {
            return out;
        }
        // Mirror and grid repeat controls take precedence over art and ruler guides; the mirror
        // axis line and the grid's controls yield to the bounding-box handles they overlap.
        if let Some(out) = self.repeat_controls(cx, ev) {
            return out;
        }
        if let Some(out) = self.guide.pointer(cx, ev) {
            return out;
        }
        let p = ev.pos;
        let m = ev.mods;
        // A press near a centre or an anchor grabs the object by it.
        self.hover = None;
        let grabbed = if matches!(ev.kind, PointerKind::Down) { Self::hover_point(cx, p).map(|(q, _)| q) } else { None };
        let grab = grabbed.unwrap_or(p);
        if matches!(ev.kind, PointerKind::Move) && matches!(self.state, State::Idle) {
            self.hover = Self::hover_point(cx, p);
        }
        match (ev.kind, self.state.clone()) {
            (PointerKind::DoubleClick, _) => self.double_click(cx, p, m),
            (PointerKind::Down, _) => {
                if let Some(out) = self.press_widgets(cx, ev, p) {
                    return out;
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
                    self.state = State::Moving { start: p, press: p, hold: p, began: false, deselect: None, key: None };
                    return vec![Action::Exec("select.set".into(), json!({ "ids": [behind.0] }))];
                }
                let (hit, scope) = match selected_source_hit(cx, p) {
                    Some((h, sources)) => (Some(h), Some(sources)),
                    None => (hit_test(cx.doc, p, cx.selection_hit_options()), cx.isolation),
                };
                match hit {
                    Some(h) => {
                        let top = h.top_object(scope);
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
                        // Grabbed by no anchor or centre, a press on a path's stroke or beside its
                        // edge holds the path there, and that point of it snaps (#3701).
                        let hold = if grabbed.is_some() { grab } else { hold_point(cx, &h, p) };
                        self.state = State::Moving { start: grab, press: p, hold, began: false, deselect, key };
                        out
                    }
                    None => {
                        self.press_nothing(cx, p, grab, m);
                        vec![]
                    }
                }
            }
            (PointerKind::Drag, State::Moving { start, press, hold, began, .. }) => {
                self.drag_moving(cx, p, m, ev.backquote, (start, press, hold, began))
            }
            (PointerKind::Drag, State::Scaling { handle, bx, areas }) => {
                self.last_backquote = ev.backquote;
                self.last_box_pos = Some(p);
                // Scale in the box's own frame: along the objects' axes when it is rotated.
                let mut a = scale_for_drag(bx.rect, handle, bx.to_local(p), m.shift, m.alt);
                self.guides.clear();
                if let Some(t) = &self.targets {
                    (a, self.guides) = t.snap_scale(cx, &bx, handle, a, m.shift, m.alt);
                }
                let nr = a.transform_rect_bbox(bx.rect);
                self.measure = cx.transform_tools_guides.then(|| (p, cx.size_label(nr.width(), nr.height())));
                // With Use Preview Bounds the box is the visual box, whose margins around the art
                // (strokes, geometry effects) scale with it or stay. `object.setBounds` then gives the
                // visual box the dragged size and keeps its opposite handle (with Alt, its center)
                // where it is. A drag that flips the box scales by the matrix.
                let [sx, _, _, sy, _, _] = a.as_coeffs();
                let (cmd, mut params) = if !ev.backquote && cx.preview_bounds && cx.selection.anchors.is_empty() && sx > 0.0 && sy > 0.0 {
                    let reference = if m.alt { 4 } else { handle.opposite().reference() };
                    ("object.setBounds", json!({ "width": nr.width(), "height": nr.height(), "reference": reference, "copy": false }))
                } else {
                    ("object.transform", json!({ "matrix": matrix_json(bx.conjugate(a)), "copy": false }))
                };
                if areas {
                    params["typeAreas"] = json!(true);
                }
                if ev.backquote {
                    params["objects"] = json!(false);
                    params["patterns"] = json!(true);
                }
                vec![Action::Preview(cmd.into(), params)]
            }
            (PointerKind::Drag, State::Rotating { center, start, .. }) => {
                self.last_backquote = ev.backquote;
                self.last_box_pos = Some(p);
                let (a, deg) = rotate_for_drag(center, start, p, m.shift);
                self.measure = cx.transform_tools_guides.then(|| (p, format!("{:.1}°", -deg)));
                let mut params = json!({ "matrix": matrix_json(a), "copy": false });
                if ev.backquote {
                    params["objects"] = json!(false);
                    params["patterns"] = json!(true);
                }
                vec![Action::Preview("object.transform".into(), params)]
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
            (PointerKind::Up, State::Moving { began, deselect, key, .. }) => self.release_move(cx, ev.backquote, began, deselect, key),
            (PointerKind::Up, State::Scaling { .. } | State::Rotating { .. }) => {
                let mut out = if ev.backquote || self.last_backquote {
                    self.last_box_pos.map_or_else(Vec::new, |pos| self.pointer(cx, &PointerEvent { kind: PointerKind::Drag, pos, ..*ev }))
                } else {
                    vec![]
                };
                self.last_backquote = false;
                self.last_box_pos = None;
                self.state = State::Idle;
                self.measure = None;
                self.guides.clear();
                self.targets = None;
                out.push(Action::Commit);
                out
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
        if let (Some(to), State::Idle, true, true) = (self.distance, &self.state, cx.smart_guides, cx.distance_guides) {
            o.extend(distance_overlays(cx, to));
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
        if let State::GridRepeat(d) = &self.state {
            return d.cursor(cx);
        }
        if matches!(self.state, State::Idle)
            && !m.cmd
            && !m.ctrl
            && !m.alt
            && let Some(cursor) = crate::mirror::cursor(cx, p).or_else(|| crate::repeat_grid::cursor(cx, p))
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
        if selected_source_hit(cx, p).is_some() {
            return Cursor::Move;
        }
        if let Some(h) = hit_test(cx.doc, p, cx.selection_hit_options()) {
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
    let stack = objects_at(cx.doc, p, cx.selection_hit_options(), cx.isolation);
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

/// Shaper's Construction mode: a Shaper Group's original shapes are isolated.
fn construction(cx: &ToolContext) -> bool {
    cx.isolation.and_then(|id| cx.doc.node(id)).is_some_and(|n| n.name.as_deref() == Some(vectorcraft_doc::shaper::SOURCES))
}

/// Is `n` area type (text in a frame) whose frame the tools reshape? Type in perspective isn't:
/// it transforms whole.
pub(crate) fn is_area_type(n: &vectorcraft_doc::Node) -> bool {
    n.extras().perspective.is_none()
        && matches!(&n.kind, vectorcraft_doc::NodeKind::Text(t) if matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }))
}

/// Is `n` type on a path outside perspective (Direct Selection drags its path's anchors)?
pub(crate) fn is_path_type(n: &vectorcraft_doc::Node) -> bool {
    n.extras().perspective.is_none()
        && matches!(&n.kind, vectorcraft_doc::NodeKind::Text(t) if matches!(t.kind, vectorcraft_doc::TextKind::OnPath { .. }))
}

/// Can `n` be a frame of a thread: area type or type on a path, outside perspective?
pub(crate) fn is_threadable(n: &vectorcraft_doc::Node) -> bool {
    n.extras().perspective.is_none() && matches!(&n.kind, vectorcraft_doc::NodeKind::Text(t) if !matches!(t.kind, vectorcraft_doc::TextKind::Point))
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
pub(crate) fn handle_cursor(h: Handle, angle: f64) -> Cursor {
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
mod tests;

impl SelectionTool {
    fn release_move(&mut self, cx: &ToolContext, backquote: bool, began: bool, deselect: Option<NodeId>, key: Option<NodeId>) -> Vec<Action> {
        self.state = State::Idle;
        self.measure = None;
        self.guides.clear();
        self.moving = None;
        let changed_mode = self.last_move.as_ref().is_some_and(|params| params.get("objects").is_some()) != backquote;
        let last = self.last_move.take().map(|mut params| {
            if backquote {
                params["objects"] = json!(false);
                params["patterns"] = json!(true);
            } else if let Some(map) = params.as_object_mut() {
                map.remove("objects");
                map.remove("patterns");
            }
            params
        });
        match (began, deselect, key) {
            // In Construction mode, an original shape dropped wholly outside the group's
            // bounds leaves the group, in the move's undo step (`releaseOutside`).
            (true, ..) if backquote || changed_mode || (construction(cx) && last.as_ref().is_some_and(|p| p["copy"] != json!(true))) => match last {
                Some(mut params) => {
                    if construction(cx) && params["copy"] != json!(true) && !backquote {
                        params["releaseOutside"] = json!(true);
                    }
                    vec![Action::Preview("object.transform".into(), params), Action::Commit]
                }
                None => vec![Action::Commit],
            },
            (true, ..) => vec![Action::Commit],
            (false, Some(id), _) => vec![Action::Exec("select.toggle".into(), json!({ "id": id.0 }))],
            // A click on the key object again: no key.
            (false, None, Some(id)) if cx.selection.key == Some(id) => vec![Action::Exec("select.key".into(), json!({}))],
            (false, None, Some(id)) if can_be_key(cx, id) => vec![Action::Exec("select.key".into(), json!({ "id": id.0 }))],
            (false, None, _) => vec![],
        }
    }

    fn repeat_controls(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
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
                    return Some(out);
                }
                _ => return Some(vec![]),
            }
        }
        if let State::GridRepeat(mut d) = self.state.clone() {
            match ev.kind {
                PointerKind::Drag | PointerKind::Up => {
                    let mut out = d.drag(ev.pos);
                    if ev.kind == PointerKind::Up {
                        out.extend(d.finish());
                        self.state = State::Idle;
                    } else {
                        self.state = State::GridRepeat(d);
                    }
                    return Some(out);
                }
                _ => return Some(vec![]),
            }
        }
        if matches!(self.state, State::Idle)
            && ev.kind == PointerKind::Down
            && let Some(d) = crate::mirror::MirrorDrag::hit(cx, ev.pos, ev.mods)
        {
            self.state = State::Mirror(d);
            self.hover = None;
            return Some(vec![]);
        }
        if matches!(self.state, State::Idle)
            && ev.kind == PointerKind::Down
            && let Some(d) = crate::repeat_grid::GridDrag::hit(cx, ev.pos, ev.mods)
        {
            self.state = State::GridRepeat(d);
            self.hover = None;
            return Some(vec![]);
        }
        None
    }

    fn double_click(&mut self, cx: &ToolContext, p: Point, m: Mods) -> Vec<Action> {
        self.state = State::Idle;
        // A threaded port: the thread breaks there. Any other port takes the double-click
        // too, so it neither edits the type nor isolates.
        if let Some((id, port)) = crate::text::port_at(cx, p) {
            let (input, output) = crate::text::port_states(cx.doc, id, false);
            let (state, name) = if port == crate::text::Port::In { (input, "in") } else { (output, "out") };
            if state == crate::text::PortState::Threaded {
                return vec![Action::Exec("text.thread.break".into(), json!({ "id": id.0, "port": name }))];
            }
            return vec![];
        }
        if let Some(a) = corners::double_click(cx, p, false).or_else(|| pie::double_click(cx, p)).or_else(|| typewidget::double_click(cx, p)) {
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
        // A double-click edits what is drawn: not a clipping path's outline.
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

    fn press_widgets(&mut self, cx: &ToolContext, ev: &PointerEvent, p: Point) -> Option<Vec<Action>> {
        // 0. A port of selected area type or type on a path: the pointer takes its text,
        // to thread on from the out port or back from the in port.
        if let Some((from, port)) = crate::text::port_at(cx, p) {
            self.state = State::Loaded { from, before: port == crate::text::Port::In, press: None };
            return Some(vec![]);
        }
        // 1. Live Corners widgets, pie widgets and type on a path's brackets, then the
        // bounding-box handles of the current selection.
        if let Some(c) = CornerDrag::hit(cx, ev, false) {
            self.state = State::Corner(c);
            return Some(vec![]);
        }
        if let Some(d) = PieDrag::hit(cx, ev) {
            self.state = State::Pie(d);
            return Some(vec![]);
        }
        if let Some(b) = BracketDrag::hit(cx, ev) {
            self.state = State::Bracket(b);
            return Some(vec![]);
        }
        if let Some(a) = shaperwidget::click(cx, p) {
            self.state = State::Idle;
            return Some(vec![a]);
        }
        // The type widget answers a double-click only: its clicks neither drag nor deselect.
        if typewidget::over_widget(cx, p) {
            self.state = State::Idle;
            return Some(vec![]);
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
                    self.last_backquote = false;
                    self.last_box_pos = None;
                    self.state = State::Scaling { handle, bx, areas };
                    return Some(vec![Action::Begin(label.into())]);
                }
                Some(BoxHit::Rotate) => {
                    self.last_backquote = false;
                    self.last_box_pos = None;
                    self.state = State::Rotating { center: bx.center(), start: p };
                    return Some(vec![Action::Begin("Rotate".into())]);
                }
                None => {}
            }
        }
        None
    }
}

impl SelectionTool {
    /// A press at `p` that hits no object starts a marquee, except where a clipping mask hides
    /// the art of a selected object (picked in the Layers panel, say): there, without Shift, it
    /// grabs the selection by that art (at `grab`), as a press on its visible part does.
    fn press_nothing(&mut self, cx: &ToolContext, p: Point, grab: Point, m: Mods) {
        self.state = if !m.shift && selected_hidden_at(cx.doc, p, cx.selection_hit_options(), &cx.selection.objects).is_some() {
            State::Moving { start: grab, press: p, hold: grab, began: false, deselect: None, key: None }
        } else {
            State::Marquee { start: p, cur: p, toggle: m.shift, enclosed: false }
        };
    }

    fn drag_moving(
        &mut self,
        cx: &ToolContext,
        p: Point,
        m: Mods,
        backquote: bool,
        (start, press, hold, began): (Point, Point, Point, bool),
    ) -> Vec<Action> {
        let mut out = vec![];
        if !began {
            if p.distance(press) < Self::drag_threshold(cx) {
                return out;
            }
            out.push(Action::Begin(if m.alt { "Copy".into() } else { "Move".into() }));
            self.moving = Some(MoveSnap::new(cx));
        }
        let mut d = move_delta(start, p, m.shift, cx.constrain_angle);
        let unsnapped = d;
        if let Some(snap) = &self.moving {
            (d, self.guides) = if m.shift { snap.snap_along(cx, hold, d, m.alt) } else { snap.snap(cx, hold, d, m.alt) };
        }
        // Shift has already constrained `d`: any change is a snap.
        self.snapped = (d - unsnapped).hypot() > 1e-9;
        self.state = State::Moving { start, press, hold, began: true, deselect: None, key: None };
        self.measure = cx.measurement_labels.then(|| (p, cx.offset_label(d.x, d.y)));
        let mut params = json!({ "matrix": matrix_json(Affine::translate(d)), "copy": m.alt });
        if backquote {
            params["objects"] = json!(false);
            params["patterns"] = json!(true);
        }
        self.last_move = Some(params.clone());
        out.push(Action::Preview("object.transform".into(), params));
        out
    }
}
