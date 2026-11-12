//! Shape tools: Rectangle, Rounded Rectangle, Ellipse, Polygon, Star, Line Segment.
//!
//! Drag draws (Shift constrains to square/circle/45°, Alt draws from the centre, Space held moves
//! the shape being drawn); a click without a drag asks the UI for the size dialog (like
//! Illustrator). ↑/↓ during a polygon/star drag change the side/point count; during a star drag,
//! Ctrl/Cmd keeps the inner radius where it is while the outer one follows the pointer, and Alt
//! keeps the star's shoulders straight (#812). The start point and
//! the dragged corner snap to Smart Guides ([`DrawSnap`]), hovering before the press too.
//! Escape cancels the drag and puts back the options its keys and Ctrl/Cmd changed.
//!
//! The selection keeps its bounding box and its Live Shape widgets on show with the Rectangle,
//! Rounded Rectangle, Ellipse, Polygon and Star tools, so a shape just drawn can be adjusted at
//! once: a drag on a live rectangle's or polygon's corner widget rounds its corners and a drag on
//! a live ellipse's pie widget sets its pie angle, as with the Selection tool; a drag on one of the
//! box's eight handles scales the selection, a press on the selection's centre point (Show Center)
//! moves it, and one just outside a corner of its box turns it about that centre, as the Selection
//! tool's handles, centre point and rotate zone do; a press anywhere else draws a new shape (#5477).

use serde_json::{Value, json};
use vectorcraft_doc::OrientedBox;
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::bbox::{Handle, move_delta, rotate_for_drag, scale_for_drag};
use crate::corners::{self, CornerDrag};
use crate::guides::{DrawSnap, Leave, PointSnap, Targets, square};
use crate::pie::{self, PieDrag};
use crate::select::{MoveSnap, handle_cursor, hit_center, is_area_type, matrix_json, rotate_zone, shown_box};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The largest corner radius the Rounded Rectangle tool draws, points: the range of General ›
/// Corner Radius, which the tool's radius is (its `prefs.list` spec), so ↑ and a Tool Options
/// value stop there too.
pub const MAX_CORNER_RADIUS: f64 = 1296.0;

/// How far (screen points, along each axis) from a bounding-box handle's centre a shape tool's
/// press scales the selection: half the drawn handle square, which is 6 points wide at the default
/// Selection & Anchor Display › Size 3 and a point wider or narrower per step. The Selection tool
/// takes 5 points.
fn handle_reach(cx: &ToolContext) -> f64 {
    (6.0 + f64::from(cx.anchor_size.clamp(1, 7)) - 3.0) / 2.0
}

/// A document offset `d` in screen points: across and down the window.
fn screen_offset(cx: &ToolContext, d: Vec2) -> (f64, f64) {
    match cx.screen {
        Some(s) if s.right.hypot2() > 0.0 && s.down.hypot2() > 0.0 => (d.dot(s.right) / s.right.hypot2(), d.dot(s.down) / s.down.hypot2()),
        _ => (d.x * cx.zoom, d.y * cx.zoom),
    }
}

/// Does tool `id` keep the selection's bounding box and Live Shape widgets on show and let its
/// corner and pie widgets be dragged (#5477)? The shape tools that draw a box: not Line Segment.
pub fn shows_shape_widgets(id: &str) -> bool {
    matches!(id, "rectangle" | "roundedRectangle" | "ellipse" | "polygon" | "star")
}

/// A drag on a Live Shape widget of the selection, taken by a shape tool.
enum WidgetDrag {
    Corner(CornerDrag),
    Pie(PieDrag),
}

/// A drag of the selection itself, taken by a shape tool: a handle of its box scales it, the centre
/// point moves it and the zone just outside a corner of its box turns it, as the Selection tool's
/// drags do (#5477).
enum SelectionDrag {
    /// Scaling the selection by `handle` of its box `bx`, as it was when the press came. `areas`:
    /// area type is selected, and its frame resizes while its type keeps its size. `snap` holds
    /// what the moved handle snaps to. `last`: where the last drag was, and whether Backtick was
    /// down then.
    Scale { handle: Handle, bx: OrientedBox, areas: bool, snap: Box<PointSnap>, last: Option<(Point, bool)> },
    /// Moving the selection by the centre point `hold`: `start` is where the press was, and the
    /// move has begun once the pointer left the drag threshold.
    Move { start: Point, hold: Point, began: bool, snap: Box<MoveSnap> },
    /// Turning the selection about `center` from where the press was.
    Rotate { center: Point, start: Point },
}

pub struct ShapeTool {
    id: &'static str,
    start: Option<Point>,
    last: Point,
    mods: Mods,
    began: bool,
    pub sides: u32,
    pub points: u32,
    pub corner_radius: f64,
    /// Star inner radius as a fraction of the outer radius.
    pub star_ratio: f64,
    /// The star's inner radius held while Ctrl/Cmd is down in a drag.
    inner: Option<f64>,
    /// Line Segment Tool Options: the length and angle (degrees) a click's dialog opens with, and
    /// Fill Line (new lines take the current fill).
    pub line_length: f64,
    pub line_angle: f64,
    pub fill_line: bool,
    snap: DrawSnap,
    /// The options a drag's keys and Ctrl/Cmd change, as they were when the drag began: Escape
    /// puts them back.
    before_drag: Option<DragOptions>,
    /// A widget of the selection is being dragged instead of a shape drawn.
    widget: Option<WidgetDrag>,
    /// The selection is being moved or turned instead of a shape drawn.
    selection: Option<SelectionDrag>,
    /// The Smart Guides a move of the selection shows.
    guides: Vec<Overlay>,
    /// The measurement label of a move or a turn: where it goes and what it reads.
    measure: Option<(Point, String)>,
    /// The move of the selection snapped to a point, a guide or the grid at its last step: the
    /// arrow turns hollow, as the Selection tool's does.
    snapped: bool,
}

/// The options that keys and Ctrl/Cmd change while a shape is dragged out.
#[derive(Clone, Copy)]
struct DragOptions {
    sides: u32,
    points: u32,
    corner_radius: f64,
    star_ratio: f64,
}

/// The inner-to-outer radius ratio that keeps a star of `points` points' shoulders straight (each
/// point's sides in line with the next points' sides: a regular star polygon {n/2}); none for
/// fewer than five points, whose shoulders can't line up.
fn straight_ratio(points: u32) -> Option<f64> {
    let n = f64::from(points);
    (points >= 5).then(|| (2.0 * std::f64::consts::PI / n).cos() / (std::f64::consts::PI / n).cos())
}

impl ShapeTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "roundedRectangle" => "roundedRectangle",
            "ellipse" => "ellipse",
            "polygon" => "polygon",
            "star" => "star",
            "lineSegment" => "lineSegment",
            _ => "rectangle",
        };
        Self {
            id,
            start: None,
            last: Point::ZERO,
            mods: Mods::default(),
            began: false,
            sides: 6,
            points: 5,
            corner_radius: 12.0,
            star_ratio: 0.5,
            inner: None,
            snap: DrawSnap::default(),
            line_length: 100.0,
            line_angle: 0.0,
            fill_line: false,
            before_drag: None,
            widget: None,
            selection: None,
            guides: vec![],
            measure: None,
            snapped: false,
        }
    }

    fn drag_options(&self) -> DragOptions {
        DragOptions { sides: self.sides, points: self.points, corner_radius: self.corner_radius, star_ratio: self.star_ratio }
    }

    /// Put back the options a canceled drag changed.
    fn restore_drag_options(&mut self, o: DragOptions) {
        self.sides = o.sides;
        self.points = o.points;
        self.corner_radius = o.corner_radius;
        self.star_ratio = o.star_ratio;
    }

    /// A press on a corner or pie widget of the selection: the drag adjusts it.
    fn press_widget(&mut self, cx: &ToolContext, ev: &PointerEvent) -> bool {
        if !shows_shape_widgets(self.id) {
            return false;
        }
        self.widget = CornerDrag::hit(cx, ev, false).map(WidgetDrag::Corner).or_else(|| PieDrag::hit(cx, ev).map(WidgetDrag::Pie));
        self.widget.is_some()
    }

    /// Is `p` over a corner or pie widget of the selection that a press would drag?
    fn over_widget(&self, cx: &ToolContext, p: Point) -> bool {
        shows_shape_widgets(self.id) && (corners::over_widget(cx, p, false) || pie::over_widget(cx, p))
    }

    /// The selection's box, while this tool shows it (the tools that draw a box, with a selection
    /// and no anchors picked on their own).
    fn box_of(&self, cx: &ToolContext) -> Option<vectorcraft_doc::OrientedBox> {
        shows_shape_widgets(self.id).then(|| shown_box(cx)).flatten()
    }

    /// Is `p` in the zone just outside a corner of the selection's box, where a press turns the
    /// selection about its centre ([`crate::select::rotate_zone`])? On the corner itself the Live
    /// Corners widget rounds it (#5477).
    fn over_rotate(&self, cx: &ToolContext, p: Point) -> Option<Point> {
        rotate_zone(cx, &self.box_of(cx)?, p)
    }

    /// The handle of the selection's box under `p`, with the box: a press there scales the
    /// selection, as with the Selection tool. A shape tool takes only the drawn handle square
    /// ([`handle_reach`]), which stays square to the screen when the box or the view is turned, so
    /// a press just beside a handle still draws a new shape.
    fn over_handle(&self, cx: &ToolContext, p: Point) -> Option<(Handle, OrientedBox)> {
        let bx = self.box_of(cx)?;
        let reach = handle_reach(cx);
        Handle::ALL
            .into_iter()
            .find(|h| {
                let (x, y) = screen_offset(cx, p - bx.to_doc() * h.pos(bx.rect));
                x.abs() <= reach && y.abs() <= reach
            })
            .map(|h| (h, bx))
    }

    /// Is `p` over the centre point of a selected object (Attributes › Show Center), where a press
    /// moves the selection? The point shows whatever View › Show Bounding Box says, so the press
    /// takes it too.
    fn over_center(&self, cx: &ToolContext, p: Point) -> bool {
        shows_shape_widgets(self.id) && hit_center(cx, p, cx.point_tol()).is_some()
    }

    /// A press on a handle of the selection's box, on its centre point or just outside a corner of
    /// its box: the drag scales, moves or turns the selection instead of drawing a new shape
    /// (#5477). The handles come first, as with the Selection tool. A scale or a turn begins with
    /// the press, as the Selection tool's do; a move begins once the pointer leaves the drag
    /// threshold.
    fn press_selection(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        if !shows_shape_widgets(self.id) {
            return None;
        }
        self.selection = if let Some((handle, bx)) = self.over_handle(cx, ev.pos) {
            let is_area = |id: &vectorcraft_doc::NodeId| cx.doc.node(*id).is_some_and(is_area_type);
            let areas = cx.selection.objects.iter().any(is_area);
            Some(SelectionDrag::Scale { handle, bx, areas, snap: Box::new(PointSnap::new(cx, || Targets::for_scale(cx))), last: None })
        } else {
            match hit_center(cx, ev.pos, cx.point_tol()) {
                Some((_, hold)) => Some(SelectionDrag::Move { start: ev.pos, hold, began: false, snap: Box::new(MoveSnap::new(cx)) }),
                None => self.over_rotate(cx, ev.pos).map(|center| SelectionDrag::Rotate { center, start: ev.pos }),
            }
        };
        self.selection.as_ref()?;
        self.snap.clear();
        self.guides.clear();
        self.measure = None;
        Some(match self.selection {
            // Only area type: the step resizes type areas; anything else scales.
            Some(SelectionDrag::Scale { areas, .. }) => {
                let only_areas = areas && cx.selection.objects.iter().all(|id| cx.doc.node(*id).is_some_and(is_area_type));
                vec![Action::Begin(if only_areas { "Resize Type Area" } else { "Scale" }.into())]
            }
            Some(SelectionDrag::Rotate { .. }) => vec![Action::Begin("Rotate".into())],
            _ => vec![],
        })
    }

    /// The pointer and keys while the selection is moved or turned: `None` when it is not.
    fn selection_pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        let d = self.selection.as_mut()?;
        let out = match (ev.kind, d) {
            (PointerKind::Drag, SelectionDrag::Move { start, hold, began, snap }) => {
                let mut out = vec![];
                if !*began {
                    if ev.pos.distance(*start) < cx.tol(3.0) {
                        return Some(out);
                    }
                    *began = true;
                    out.push(Action::Begin(if ev.mods.alt { "Copy".into() } else { "Move".into() }));
                }
                let shift = ev.mods.shift;
                let unsnapped = move_delta(*start, ev.pos, shift, cx.constrain_angle);
                let (delta, guides) =
                    if shift { snap.snap_along(cx, *hold, unsnapped, ev.mods.alt) } else { snap.snap(cx, *hold, unsnapped, ev.mods.alt) };
                // Shift has already constrained the move: any change is a snap.
                self.snapped = (delta - unsnapped).hypot() > 1e-9;
                self.guides = guides;
                self.measure = cx.measurement_labels.then(|| (ev.pos, cx.offset_label(delta.x, delta.y)));
                out.push(Action::Preview("object.transform".into(), json!({"matrix": matrix_json(Affine::translate(delta)), "copy": ev.mods.alt})));
                out
            }
            (PointerKind::Drag, SelectionDrag::Rotate { center, start }) => {
                let (a, deg) = rotate_for_drag(*center, *start, ev.pos, ev.mods.shift);
                self.measure = cx.transform_tools_guides.then(|| (ev.pos, format!("{:.1}°", -deg)));
                vec![Action::Preview("object.transform".into(), json!({"matrix": matrix_json(a), "copy": false}))]
            }
            (PointerKind::Drag, SelectionDrag::Scale { handle, bx, areas, snap, last }) => {
                // As the Selection tool scales: in the box's own frame, the moved handle snapped,
                // and with Use Preview Bounds by the visual box's size unless the drag flips it.
                *last = Some((ev.pos, ev.backquote));
                let m = ev.mods;
                let a = scale_for_drag(bx.rect, *handle, bx.to_local(ev.pos), m.shift, m.alt);
                let (a, guides) = snap.snap_scale(cx, bx, *handle, a, m.shift, m.alt);
                self.guides = guides;
                let nr = a.transform_rect_bbox(bx.rect);
                self.measure = cx.transform_tools_guides.then(|| (ev.pos, cx.size_label(nr.width(), nr.height())));
                let [sx, _, _, sy, _, _] = a.as_coeffs();
                let (cmd, mut params) = if !ev.backquote && cx.preview_bounds && sx > 0.0 && sy > 0.0 {
                    let reference = if m.alt { 4 } else { handle.opposite().reference() };
                    ("object.setBounds", json!({ "width": nr.width(), "height": nr.height(), "reference": reference, "copy": false }))
                } else {
                    ("object.transform", json!({ "matrix": matrix_json(bx.conjugate(a)), "copy": false }))
                };
                if *areas {
                    params["typeAreas"] = json!(true);
                }
                if ev.backquote {
                    params["objects"] = json!(false);
                    params["patterns"] = json!(true);
                }
                vec![Action::Preview(cmd.into(), params)]
            }
            (PointerKind::Up, SelectionDrag::Move { began, .. }) => {
                let began = *began;
                self.selection = None;
                self.guides.clear();
                self.measure = None;
                self.snapped = false;
                if began { vec![Action::Commit] } else { vec![] }
            }
            (PointerKind::Up, SelectionDrag::Scale { last, .. }) => {
                // Backtick pressed or let go after the last drag: the scale is previewed again at
                // that drag's position with the key as it is at the release, as the Selection tool
                // does.
                let again = last.filter(|(_, backquote)| *backquote != ev.backquote).map(|(pos, _)| pos);
                let mut out =
                    again.and_then(|pos| self.selection_pointer(cx, &PointerEvent { kind: PointerKind::Drag, pos, ..*ev })).unwrap_or_default();
                self.selection = None;
                self.guides.clear();
                self.measure = None;
                out.push(Action::Commit);
                out
            }
            (PointerKind::Up, SelectionDrag::Rotate { .. }) => {
                self.selection = None;
                self.guides.clear();
                self.measure = None;
                vec![Action::Commit]
            }
            _ => vec![],
        };
        Some(out)
    }

    /// Escape during a drag of the selection turns it back, as the Selection tool's does.
    fn cancel_selection(&mut self) -> Vec<Action> {
        self.selection = None;
        self.guides.clear();
        self.measure = None;
        self.snapped = false;
        vec![Action::Cancel]
    }

    /// The pointer and keys while a widget is dragged: `None` when none is.
    fn widget_pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        let w = self.widget.as_mut()?;
        Some(match ev.kind {
            PointerKind::Drag => match w {
                WidgetDrag::Corner(c) => c.drag(cx, ev.pos),
                WidgetDrag::Pie(d) => d.drag(cx, ev.pos),
            },
            PointerKind::Up => match self.widget.take() {
                Some(WidgetDrag::Corner(c)) => c.finish(),
                Some(WidgetDrag::Pie(d)) => d.finish(),
                None => vec![],
            },
            _ => vec![],
        })
    }

    /// Ctrl/Cmd in a star drag from `start` to `p`, where the last drag showed a star with the inner
    /// radius `shown`: once the star has begun, the inner radius stays as the star showed it when
    /// the key went down, so the ratio follows the outer radius (and stays so once the key is let
    /// go). When the key goes down before the star has begun, the inner radius stays as the first
    /// star the drag shows has it.
    fn hold_inner(&mut self, shown: f64, start: Point, p: Point, m: Mods) {
        let outer = (p - start).hypot();
        match (m.cmd, self.inner) {
            (true, None) if self.began => self.inner = Some(shown),
            (false, Some(_)) => self.inner = None,
            _ => {}
        }
        if let Some(inner) = self.inner
            && outer > 0.0
            && (inner / outer).is_finite()
        {
            self.star_ratio = (inner / outer).clamp(0.01, 1.0);
        }
    }

    fn label(&self) -> &'static str {
        match self.id {
            "roundedRectangle" => "Rounded Rectangle",
            "ellipse" => "Ellipse",
            "polygon" => "Polygon",
            "star" => "Star",
            "lineSegment" => "Line",
            _ => "Rectangle",
        }
    }

    /// How the dragged point keeps to the start: a polygon's or star's radius goes anywhere, a
    /// line is a segment (Shift: 45° steps), the rest are boxes (Shift: a square).
    fn leave(&self, cx: &ToolContext, start: Point, m: Mods) -> Option<Leave> {
        match self.id {
            "polygon" | "star" => None,
            "lineSegment" => Some(Leave::segment(cx, start, m.shift)),
            // With a Constrain Angle, Shift's square is taken along the rotated axes
            // ([`Self::command_at`]), not the page's diagonal.
            _ => Some(Leave::diagonal(start, m.shift && vectorcraft_geom::constrain_axes(cx.constrain_angle).is_none())),
        }
    }

    /// The command for a drag from `start` to `p` (snapped: a line's end already keeps Shift's
    /// angle).
    pub fn command(&self, start: Point, p: Point, m: Mods) -> (String, Value) {
        self.command_at(start, p, m, 0.0)
    }

    /// [`Self::command`] with General › Constrain Angle `base` (degrees, counter-clockwise):
    /// rectangles, rounded rectangles and ellipses are drawn along the x and y axes turned by it,
    /// as the reference app draws new objects along the rotated axes
    /// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/arrange-objects/rotate-objects-by-specific-angles.html>).
    pub fn command_at(&self, start: Point, p: Point, m: Mods, base: f64) -> (String, Value) {
        match self.id {
            "polygon" | "star" => {
                let v = p - start;
                let r = v.hypot().max(0.01);
                // Shift keeps the base orientation: upright, turned by the Constrain Angle.
                let rot = if m.shift { vectorcraft_geom::constrain_rotation(base).unwrap_or(0.0) } else { (v.atan2().to_degrees() + 90.0) % 360.0 };
                if self.id == "polygon" {
                    ("shape.polygon".into(), json!({ "cx": start.x, "cy": start.y, "radius": r, "sides": self.sides, "rotation": rot }))
                } else {
                    let ratio = if m.alt { straight_ratio(self.points).unwrap_or(self.star_ratio) } else { self.star_ratio };
                    (
                        "shape.star".into(),
                        json!({ "cx": start.x, "cy": start.y, "radius1": r, "radius2": r * ratio, "points": self.points, "rotation": rot }),
                    )
                }
            }
            "lineSegment" => {
                let (a, b) = if m.alt { (start - (p - start), p) } else { (start, p) };
                let mut v = json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y });
                if self.fill_line {
                    v["fill"] = json!(true);
                }
                ("shape.line".into(), v)
            }
            _ => {
                let cmd = if self.id == "ellipse" { "shape.ellipse" } else { "shape.rectangle" };
                let mut v = box_params(start, p, m, base);
                if self.id == "roundedRectangle" {
                    v["radius"] = json!(self.corner_radius);
                }
                (cmd.into(), v)
            }
        }
    }
}

/// The box a drag from `start` to `p` draws (from its centre with Alt, square with Shift),
/// measured along the page axes turned by Constrain Angle `base`: the box in those axes, and the
/// axes (`None` for the page's own: 0° or a whole turn, where the box is on the page exactly).
/// The drawn shape and its size label both come from it.
pub(crate) fn turned_box(start: Point, p: Point, m: Mods, base: f64) -> (Rect, Option<Affine>) {
    match vectorcraft_geom::constrain_axes(base) {
        None => (drag_rect(start, p, m), None),
        Some(axes) => {
            let back = axes.inverse();
            (drag_rect(back * start, back * p, m), Some(axes))
        }
    }
}

/// The `{x, y, width, height, rotation?}` params of the box a drag from `start` to `p` draws along
/// Constrain Angle `base` ([`turned_box`]): the box's page position at 0° or a whole turn, else
/// the turned box's centre out on the page again with the rotation that turns it about it.
pub(crate) fn box_params(start: Point, p: Point, m: Mods, base: f64) -> Value {
    match turned_box(start, p, m, base) {
        (r, None) => json!({ "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height() }),
        (r, Some(axes)) => {
            let c = axes * r.center();
            let rotation = -vectorcraft_geom::normalize_deg(base);
            json!({ "x": c.x - r.width() / 2.0, "y": c.y - r.height() / 2.0, "width": r.width(), "height": r.height(), "rotation": rotation })
        }
    }
}

/// Space held once a shape drag has begun moves the shape being drawn instead of sizing it: `start`
/// moves as far as the pointer did since `last`, which becomes the pointer.
pub(crate) fn space_moves(start: &mut Point, last: &mut Point, ev: &PointerEvent, began: bool) {
    if ev.mods.space && began {
        *start += ev.pos - *last;
    }
    *last = ev.pos;
}

/// The rectangle a drag from `start` to `p` draws: Shift makes it a square, Alt draws it from its
/// centre.
pub(crate) fn drag_rect(start: Point, p: Point, m: Mods) -> Rect {
    let d = if m.shift { square(p - start) } else { p - start };
    if m.alt { Rect::from_points(start - d, start + d) } else { Rect::from_points(start, start + d) }
}

impl Tool for ShapeTool {
    fn id(&self) -> &'static str {
        self.id
    }
    /// A shape being drawn, a Live Shape widget dragged, or the selection moved or turned is a
    /// gesture in progress.
    fn busy(&self) -> bool {
        self.start.is_some() || self.widget.is_some() || self.selection.is_some()
    }
    /// A widget or selection drag takes the pointer; otherwise a press draws, unless it lands on a
    /// handle of the selection's box, its centre point or the rotate zone just outside its box's
    /// corner.
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if let Some(out) = self.widget_pointer(cx, ev) {
            return out;
        }
        if let Some(out) = self.selection_pointer(cx, ev) {
            return out;
        }
        match ev.kind {
            PointerKind::Move => {
                // Over a widget a press would drag, no Smart Guide for a new shape's start.
                if self.over_widget(cx, ev.pos)
                    || self.over_handle(cx, ev.pos).is_some()
                    || self.over_center(cx, ev.pos)
                    || self.over_rotate(cx, ev.pos).is_some()
                {
                    self.snap.clear();
                } else {
                    self.snap.hover(cx, ev.pos, &[], None);
                }
                vec![]
            }
            PointerKind::Down => {
                if self.press_widget(cx, ev) {
                    self.snap.clear();
                    return vec![];
                }
                if let Some(out) = self.press_selection(cx, ev) {
                    return out;
                }
                let p = self.snap.press(cx, ev.pos, &[], None);
                self.start = Some(p);
                self.last = ev.pos;
                self.began = false;
                self.inner = None;
                self.before_drag = None;
                vec![]
            }
            PointerKind::Drag => {
                let Some(mut s) = self.start else { return vec![] };
                let pos = self.snap.drag(cx, ev.pos, self.leave(cx, s, ev.mods).as_ref());
                let ev = &PointerEvent { pos, ..*ev };
                // The inner radius of the star the last drag showed, with the ratio it drew with:
                // the canvas sends a drag only once the pointer moves, so a key pressed in between
                // first arrives with the next position.
                let ratio = if self.mods.alt { straight_ratio(self.points).unwrap_or(self.star_ratio) } else { self.star_ratio };
                let shown = (self.last - s).hypot() * ratio;
                space_moves(&mut s, &mut self.last, ev, self.began);
                self.start = Some(s);
                self.mods = ev.mods;
                if self.id == "star" {
                    self.hold_inner(shown, s, ev.pos, ev.mods);
                }
                let mut out = vec![];
                if !self.began {
                    if ev.pos.distance(s) < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    self.before_drag = Some(self.drag_options());
                    out.push(Action::Begin(self.label().into()));
                }
                let (c, v) = self.command_at(s, ev.pos, ev.mods, cx.constrain_angle);
                out.push(Action::Preview(c, v));
                out
            }
            PointerKind::Up => {
                let Some(s) = self.start.take() else { return vec![] };
                self.snap.clear();
                self.inner = None;
                self.before_drag = None;
                if self.began {
                    self.began = false;
                    vec![Action::Commit]
                } else {
                    // Alt (draw from centre) is passed on, so the size dialog can centre the shape on the click.
                    let args = if ev.mods.alt { json!({ "x": s.x, "y": s.y, "alt": true }) } else { json!({ "x": s.x, "y": s.y }) };
                    vec![Action::Dialog(self.id.into(), args)]
                }
            }
            _ => vec![],
        }
    }
    /// ↑/↓ change a polygon's sides, a star's points or the rounded rectangle's radius while it is
    /// drawn; Escape turns a widget or selection drag back.
    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        // A drag of the selection: Escape turns it back, as with the Selection tool.
        if self.selection.is_some() {
            return if key == ToolKey::Escape { self.cancel_selection() } else { vec![] };
        }
        // A widget drag: Up and Down step a corner's kind; Escape cancels it, as with the
        // Selection tool.
        if let Some(w) = &mut self.widget {
            return match (key, w) {
                (ToolKey::Up | ToolKey::Down, WidgetDrag::Corner(c)) => c.step_kind(key == ToolKey::Up),
                (ToolKey::Escape, _) => match self.widget.take() {
                    Some(WidgetDrag::Corner(c)) if c.adjusting() => vec![Action::Cancel],
                    Some(WidgetDrag::Pie(d)) if d.adjusting() => vec![Action::Cancel],
                    _ => vec![],
                },
                _ => vec![],
            };
        }
        let Some(s) = self.start.filter(|_| self.began) else { return vec![] };
        let changed = match (self.id, key) {
            ("polygon", ToolKey::Up) => {
                self.sides = (self.sides + 1).min(1000);
                true
            }
            ("polygon", ToolKey::Down) => {
                self.sides = self.sides.saturating_sub(1).max(3);
                true
            }
            ("star", ToolKey::Up) => {
                self.points = (self.points + 1).min(1000);
                true
            }
            ("star", ToolKey::Down) => {
                self.points = self.points.saturating_sub(1).max(3);
                true
            }
            ("roundedRectangle", ToolKey::Up) => {
                self.corner_radius = (self.corner_radius + 1.0).min(MAX_CORNER_RADIUS);
                true
            }
            ("roundedRectangle", ToolKey::Down) => {
                self.corner_radius = (self.corner_radius - 1.0).max(0.0);
                true
            }
            _ => false,
        };
        if changed {
            let (c, v) = self.command_at(s, self.last, self.mods, cx.constrain_angle);
            vec![Action::Preview(c, v)]
        } else if key == ToolKey::Escape {
            self.start = None;
            self.began = false;
            self.inner = None;
            if let Some(o) = self.before_drag.take() {
                self.restore_drag_options(o);
            }
            self.snap.clear();
            vec![Action::Cancel]
        } else {
            vec![]
        }
    }
    /// A drag of a handle of the selection's box: the canvas keeps drawing the box at the changing
    /// bounds, as for the Selection tool.
    fn resizing(&self) -> bool {
        matches!(self.selection, Some(SelectionDrag::Scale { .. }))
    }
    /// Adjusting a widget (the canvas hides the bounding box meanwhile, as for the Selection tool).
    fn transforming(&self) -> bool {
        match &self.widget {
            Some(WidgetDrag::Corner(c)) => c.adjusting(),
            Some(WidgetDrag::Pie(d)) => d.adjusting(),
            None => match &self.selection {
                Some(SelectionDrag::Move { began, .. }) => *began,
                Some(SelectionDrag::Rotate { .. } | SelectionDrag::Scale { .. }) => true,
                None => false,
            },
        }
    }
    /// ↑/↓ step a corner's kind while its widget is dragged, and Escape ends a drag of the
    /// selection.
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        match (&self.widget, &self.selection) {
            (Some(WidgetDrag::Corner(_)), _) => matches!(key, ToolKey::Up | ToolKey::Down),
            (_, Some(_)) => key == ToolKey::Escape,
            _ => false,
        }
    }
    /// The widget being dragged, the Smart Guides of the shape being drawn or the selection being
    /// moved, and its size, angle or offset label.
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        match &self.widget {
            Some(WidgetDrag::Corner(c)) => return c.overlays(cx),
            Some(WidgetDrag::Pie(d)) => return d.overlays(cx),
            None => {}
        }
        let mut o = self.snap.guides().to_vec();
        o.extend(self.guides.iter().cloned());
        if let Some((p, text)) = &self.measure {
            o.push(Overlay::Measure { p: *p, text: text.clone() });
        }
        if let Some(s) = self.start.filter(|_| self.began && cx.measurement_labels) {
            let p = self.last + Vec2::new(cx.tol(12.0), cx.tol(12.0));
            let text = if self.id == "lineSegment" {
                cx.segment_measure_label(s, self.last)
            } else {
                // A box's size as drawn (from its centre with Alt, square with Shift).
                match self.id {
                    "star" => {
                        let outer = (self.last - s).hypot().max(0.01);
                        // The same ratio `command_at` draws with, so the label matches the preview
                        // (Ctrl/Cmd's held radius is already in `star_ratio`, clamped; Alt overrides it).
                        let ratio = if self.mods.alt { straight_ratio(self.points).unwrap_or(self.star_ratio) } else { self.star_ratio };
                        cx.star_radii_label(outer, outer * ratio)
                    }
                    _ => {
                        let (w, h) = match self.id {
                            "polygon" => ((self.last - s).x.abs(), (self.last - s).y.abs()),
                            _ => {
                                let (r, _) = turned_box(s, self.last, self.mods, cx.constrain_angle);
                                (r.width(), r.height())
                            }
                        };
                        cx.size_label(w, h)
                    }
                }
            };
            o.push(Overlay::Measure { p, text });
        }
        o
    }
    /// The Selection tool's pointer over a Live Shape widget, a handle of the selection's box, its
    /// centre point or the rotate zone just outside its box's corner; the crosshair everywhere
    /// else, where a press draws.
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        // Over a widget a press would drag (or dragging one), the Selection tool's pointer for it.
        let idle = self.start.is_none();
        match &self.widget {
            Some(WidgetDrag::Corner(_)) => return Cursor::CornerRadius,
            Some(WidgetDrag::Pie(_)) => return Cursor::ArrowHollow,
            None => {}
        }
        // Alt-dragging the selection leaves a copy behind; otherwise the arrow, hollow while the
        // move snaps, as the Selection tool's.
        match &self.selection {
            Some(SelectionDrag::Rotate { center, .. }) => return Cursor::rotate_about_in_view(*center, p, cx.view_angle()),
            Some(SelectionDrag::Scale { handle, bx, .. }) => return handle_cursor(*handle, bx.angle - cx.view_angle()),
            Some(SelectionDrag::Move { began: true, .. }) if m.alt => return Cursor::SelectionCopy,
            Some(SelectionDrag::Move { began: true, .. }) => return if self.snapped { Cursor::ArrowHollow } else { Cursor::Arrow },
            Some(_) => return Cursor::Arrow,
            None => {}
        }
        // A press takes the corner and pie widgets first, then the box's handles, then the centre
        // point, then the zone just outside a corner; everywhere else it draws a new shape.
        if idle {
            if shows_shape_widgets(self.id) && corners::over_widget(cx, p, false) {
                return Cursor::CornerRadius;
            }
            if self.over_widget(cx, p) {
                return Cursor::ArrowHollow;
            }
            if let Some((handle, bx)) = self.over_handle(cx, p) {
                return handle_cursor(handle, bx.angle - cx.view_angle());
            }
            if self.over_center(cx, p) {
                return Cursor::Move;
            }
            if let Some(center) = self.over_rotate(cx, p) {
                return Cursor::rotate_about_in_view(center, p, cx.view_angle());
            }
        }
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        json!({
            "sides": self.sides,
            "points": self.points,
            "cornerRadius": self.corner_radius,
            "starRatio": self.star_ratio,
            "length": self.line_length,
            "angle": self.line_angle,
            "fillLine": self.fill_line,
        })
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        let finite = v.as_f64().filter(|n| n.is_finite());
        match key {
            "sides" => self.sides = v.as_u64().unwrap_or(6).clamp(3, 1000) as u32,
            "points" => self.points = v.as_u64().unwrap_or(5).clamp(3, 1000) as u32,
            "cornerRadius" => self.corner_radius = finite.unwrap_or(12.0).clamp(0.0, MAX_CORNER_RADIUS),
            "starRatio" => self.star_ratio = v.as_f64().unwrap_or(0.5).clamp(0.01, 1.0),
            "length" => self.line_length = finite.map_or(self.line_length, |n| n.abs().min(1e6)),
            "angle" => self.line_angle = finite.map_or(self.line_angle, |n| n.rem_euclid(360.0)),
            "fillLine" => self.fill_line = v.as_bool().unwrap_or(self.fill_line),
            _ => {}
        }
        // An option set while a shape is dragged out stays when Escape cancels the drag.
        if let Some(b) = self.before_drag.as_mut() {
            match key {
                "sides" => b.sides = self.sides,
                "points" => b.points = self.points,
                "cornerRadius" => b.corner_radius = self.corner_radius,
                "starRatio" => b.star_ratio = self.star_ratio,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests;
