//! The Objects on Path tool (#2545): a click on a path attaches the selected objects to it
//! (`object.objectsOnPath.attach`). A path they can attach to is highlighted under the pointer,
//! and the pointer shows [`Cursor::ObjectsOnPath`] over it. The tool's options, `pivot` (0-8, row
//! by row from the top left; 4, the center) and `rotate` (degrees), go with each attach, so they
//! set how the objects attached next attach; groups attached before keep theirs.
//!
//! The Rotate All widget: with one Objects on Path group selected, the Selection tool and this
//! tool show a round handle beside the group's first object. Dragging it turns every object of the
//! group about its attachment point by the angle the pointer turns about the first object's
//! attachment point: the angle is added to the group's Rotate (`object.objectsOnPath.options`),
//! previewed while dragging and kept as one undo step on release. Shift turns in steps of 15°, and
//! Escape cancels the drag.
//!
//! The Space widget: with one Objects on Path group of two or more objects selected, the same
//! tools show a square handle beyond the group's last object, ahead along the path. Dragging it
//! along the path sets the distance between the objects' attachment points
//! (`object.objectsOnPath.options` with `spacing` and `start`): the first object stays where it is
//! and the others move toward or away from it. With Alt (Option on macOS) the objects spread from
//! both ends toward or away from the center of an open path, or of the first anchor of a closed
//! one. The distance shows next to the pointer in the document's units, the drag is previewed and
//! kept as one undo step on release, and Escape cancels it.
//!
//! The Move All widget: with one Objects on Path group selected whose objects have room to move
//! along its path, the same tools show a diamond beyond the group's first object, below it.
//! Dragging it along the path moves every object along it as far, keeping the distances between
//! them (`object.objectsOnPath.options` with `start`). On an open path, the widget shows only when
//! the distance from the first object to the last is shorter than the path, and the objects stop
//! at its ends; on a closed path they go on round across its first anchor. The first object's
//! distance along the path shows next to the pointer in the document's units, the drag is
//! previewed and kept as one undo step on release, and Escape cancels it.
//!
//! The Select/Move widgets: with one Objects on Path group selected, the same tools show a small
//! ring on each object's attachment point, highlighted under the pointer. A click on one chooses
//! that object (`object.objectsOnPath.selectMember`), and its ring is filled; the group stays
//! selected, and a click on the object's art still selects the group. Dragging a ring along the
//! path moves its object to the place of the object nearest the pointer, and the objects between
//! move one place toward where it was (`object.objectsOnPath.moveMember`): only their order along
//! the path changes, and hidden and locked objects keep their places. On an open path a pointer beyond an end takes the place at that end; on a
//! closed path the drag goes on round across the first anchor. The drag is previewed and kept as
//! one undo step on release, and Escape cancels it.
//!
//! Behavior: <https://helpx.adobe.com/illustrator/using/objects-on-path.html>,
//! <https://helpx.adobe.com/illustrator/desktop/manage-objects/arrange-objects/rearrange-and-edit-objects-on-a-path.html>.

use serde_json::{Value, json};
use vectorcraft_doc::blend::Spine;
use vectorcraft_doc::objects_on_path::{
    CENTER, Member, ObjectsOnPath, attachable_path, is_objects_on_path, layout, nth_length, path_length, stop, stops,
};
use vectorcraft_doc::{Document, Node, NodeId, Selection};
use vectorcraft_geom::{BezPath, Point, Vec2};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The highlight of the path under the pointer.
const HIGHLIGHT: [u8; 3] = [0x4f, 0x9d, 0xff];

/// The Rotate All widget's center sits this far (screen px) beyond the first object, on the side
/// its top faces.
const ROTATE_ALL_OFFSET_PX: f64 = 16.0;
/// The Rotate All widget's radius (screen px), as the canvas draws it.
pub const ROTATE_ALL_RADIUS_PX: f32 = 4.0;
/// How near (screen px) the pointer must be to hit the Rotate All widget.
const ROTATE_ALL_HIT_PX: f64 = 6.0;
/// The step (degrees) a Shift-drag of the Rotate All widget turns by.
const ROTATE_ALL_STEP: f64 = 15.0;

/// The Space widget's center sits this far (screen px) beyond the last object, ahead along the path.
const SPACE_OFFSET_PX: f64 = 16.0;
/// Half the side of the Space widget's square (screen px), as the canvas draws it.
pub const SPACE_HALF_PX: f32 = 3.5;
/// How near (screen px) the pointer must be to hit the Space widget.
const SPACE_HIT_PX: f64 = 6.0;

/// The Move All widget's center sits this far (screen px) beyond the first object, on the side its
/// bottom faces.
const MOVE_ALL_OFFSET_PX: f64 = 16.0;
/// Half the diagonal of the Move All widget's diamond (screen px), as the canvas draws it.
pub const MOVE_ALL_HALF_PX: f32 = 4.5;
/// How near (screen px) the pointer must be to hit the Move All widget.
const MOVE_ALL_HIT_PX: f64 = 6.0;

/// The radius (screen px) of the Select/Move widget's ring, as the canvas draws it.
pub const MEMBER_RADIUS_PX: f32 = 4.5;
/// How near (screen px) the pointer must be to hit a Select/Move widget.
const MEMBER_HIT_PX: f64 = 6.0;

/// The Objects on Path group the widgets show for at `zoom` (screen px per document point), with
/// its record, its children and its path: only when the selection is that one editable group, on
/// a path with length, and no anchors are Direct-Selected.
fn one_group<'a>(
    doc: &'a Document,
    selection: &Selection,
    zoom: f64,
) -> Option<(NodeId, &'a ObjectsOnPath, &'a [std::sync::Arc<vectorcraft_doc::Node>], &'a vectorcraft_geom::PathData)> {
    if !selection.anchors.is_empty() || !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    let [group] = selection.objects.as_slice() else { return None };
    if !doc.editable()(*group) {
        return None;
    }
    let n = doc.node(*group)?;
    let rec = n.extras().objects_on_path.as_deref()?;
    let children = n.children()?;
    let path = attachable_path(children.iter().find(|c| c.id == rec.path)?)?;
    Some((*group, rec, children.as_slice(), path))
}

/// The direction the top of the object `m` faces: the -y direction of the frame it is drawn in
/// (the document's y axis points down), so a reflected or sheared frame turns it with the art's
/// top. A unit vector in the document's axes.
fn top(m: &Member) -> Vec2 {
    let [_, _, a2, a3, _, _] = m.frame.as_coeffs();
    let len = a2.hypot(a3);
    if len.is_finite() && len > 1e-12 {
        Vec2::new(-a2 / len, -a3 / len)
    } else {
        let (sin, cos) = m.angle.sin_cos();
        Vec2::new(sin, -cos)
    }
}

/// The point `gap` (points) beyond the bounds of the object `m` (one of `children`), from its
/// attachment point `from` in the direction `dir`. `None` when it is not finite.
fn beside(doc: &Document, children: &[std::sync::Arc<Node>], m: &Member, from: Point, dir: Vec2, gap: f64) -> Option<Point> {
    let b = children.iter().find(|c| c.id == m.id)?.geometric_bounds_with(&doc.symbol_boxes())?;
    let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x0, b.y1), Point::new(b.x1, b.y1)];
    let reach = corners.iter().map(|c| (*c - from).dot(dir)).fold(0.0, f64::max);
    let at = from + dir * (reach + gap);
    (at.x.is_finite() && at.y.is_finite() && from.x.is_finite() && from.y.is_finite()).then_some(at)
}

/// The Rotate All widget of a selected Objects on Path group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotateAllWidget {
    pub group: NodeId,
    /// The first object's attachment point, in the document: a drag turns about it.
    pub pivot: Point,
    /// The widget's center in the document.
    pub at: Point,
    /// The group's Rotate (degrees).
    pub rotate: f64,
}

impl RotateAllWidget {
    /// The widget of the selection at `zoom` (screen px per document point): only when the
    /// selection is one editable Objects on Path group with at least one object on a path with
    /// length, and no anchors are Direct-Selected.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64) -> Option<Self> {
        let (group, rec, children, path) = one_group(doc, selection, zoom)?;
        let first = rec.members.first()?;
        let (pivot, _) = stop(path, rec.members.len(), rec.spacing, rec.start, 0)?;
        let at = beside(doc, children, first, pivot, top(first), ROTATE_ALL_OFFSET_PX / zoom)?;
        Some(Self { group, pivot, at, rotate: rec.rotate })
    }

    /// The widget of `cx` when `p` is within reach of it.
    fn at_point(cx: &ToolContext, p: Point) -> Option<Self> {
        Self::of(cx.doc, cx.selection, cx.zoom).filter(|w| w.at.distance(p) <= cx.tol(ROTATE_ALL_HIT_PX))
    }
}

/// Is `p` over the Rotate All widget?
pub fn over_rotate_all(cx: &ToolContext, p: Point) -> bool {
    RotateAllWidget::at_point(cx, p).is_some()
}

/// Dragging the Rotate All widget: the group's Rotate follows the angle the pointer turns about
/// the first object's attachment point, counter-clockwise on the page, over as many turns as the
/// drag makes.
#[derive(Clone, Debug)]
pub struct RotateAllDrag {
    widget: RotateAllWidget,
    start: Point,
    began: bool,
    /// The pointer's direction from the pivot at the last drag (radians, the document's axes).
    last: f64,
    /// The degrees the pointer has turned counter-clockwise on the page since the press.
    turned: f64,
    /// The Rotate the drag sets.
    rotate: f64,
    at: Point,
}

impl RotateAllDrag {
    /// The drag has left the click: the angle is changing.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// Start a drag when the press `ev` is on the Rotate All widget.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let widget = RotateAllWidget::at_point(cx, ev.pos)?;
        let d = ev.pos - widget.pivot;
        let last = d.y.atan2(d.x);
        let last = if last.is_finite() { last } else { 0.0 };
        Some(Self { widget, start: ev.pos, began: false, last, turned: 0.0, rotate: widget.rotate, at: ev.pos })
    }

    /// The pointer moved to `p`; `shift` turns in steps of 15°.
    pub fn drag(&mut self, cx: &ToolContext, p: Point, shift: bool) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Objects on Path Options".into()));
        }
        let d = p - self.widget.pivot;
        if d.hypot() > 1e-9 {
            let now = d.y.atan2(d.x);
            let step = (now - self.last + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
            if step.is_finite() {
                self.turned -= step.to_degrees();
                self.last = now;
            }
        }
        let turned = if shift { (self.turned / ROTATE_ALL_STEP).round() * ROTATE_ALL_STEP } else { self.turned };
        let r = self.widget.rotate + turned;
        self.rotate = if r.abs() > 360.0 { r % 360.0 } else { r } + 0.0;
        self.at = p;
        out.push(Action::Preview("object.objectsOnPath.options".into(), json!({ "ids": [self.widget.group.0], "rotate": self.rotate })));
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    /// The Rotate readout next to the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.began || !cx.measurement_labels {
            return vec![];
        }
        vec![Overlay::Measure { p: self.at, text: format!("{:.1}°", self.rotate) }]
    }
}

/// The Space widget of a selected Objects on Path group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceWidget {
    pub group: NodeId,
    /// The last object's attachment point, in the document.
    pub last: Point,
    /// The distance along the path to the last object's attachment point (points).
    pub last_length: f64,
    /// The widget's center in the document.
    pub at: Point,
    /// The path's direction at the last object's attachment point: a unit vector in the
    /// document's axes, in which the widget sits beyond the object.
    pub dir: Vec2,
    /// The length of the path (points).
    pub len: f64,
    /// Is the path closed?
    pub closed: bool,
    /// The number of objects on the path: 2 or more.
    pub count: usize,
    /// Where the first object attaches along the path (points from its first anchor), as drawn.
    pub start: f64,
    /// The distance along the path between neighboring objects' attachment points (points), as drawn.
    pub gap: f64,
    /// The group's Space value (`None`: Auto) and start, as saved: more than is drawn when the path
    /// is too short for them.
    pub saved: (Option<f64>, f64),
}

impl SpaceWidget {
    /// The widget of the selection at `zoom` (screen px per document point): only when the
    /// selection is one editable Objects on Path group with two or more objects on a path with
    /// length, and no anchors are Direct-Selected. It sits beyond the last object's bounds, ahead
    /// along the path's direction there.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64) -> Option<Self> {
        let (group, rec, children, path) = one_group(doc, selection, zoom)?;
        let count = rec.members.len();
        if count < 2 {
            return None;
        }
        let (len, closed) = path_length(path)?;
        let (start, gap) = layout(len, closed, count, rec.spacing, rec.start);
        let (last, angle) = stop(path, count, rec.spacing, rec.start, count - 1)?;
        let dir = Vec2::new(angle.cos(), angle.sin());
        let m = rec.members.last()?;
        let b = children.iter().find(|c| c.id == m.id)?.geometric_bounds_with(&doc.symbol_boxes())?;
        let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x0, b.y1), Point::new(b.x1, b.y1)];
        let reach = corners.iter().map(|c| (*c - last).dot(dir)).fold(0.0, f64::max);
        let at = last + dir * (reach + SPACE_OFFSET_PX / zoom);
        let finite = at.x.is_finite() && at.y.is_finite() && last.x.is_finite() && last.y.is_finite();
        let last_length = nth_length(len, closed, start, gap, count - 1);
        finite.then_some(Self { group, last, last_length, at, dir, len, closed, count, start, gap, saved: (rec.spacing, rec.start) })
    }

    /// The widget of `cx` when `p` is within reach of it.
    fn at_point(cx: &ToolContext, p: Point) -> Option<Self> {
        Self::of(cx.doc, cx.selection, cx.zoom).filter(|w| w.at.distance(p) <= cx.tol(SPACE_HIT_PX))
    }
}

/// Is `p` over the Space widget?
pub fn over_space(cx: &ToolContext, p: Point) -> bool {
    SpaceWidget::at_point(cx, p).is_some()
}

/// The distance along the path of the Objects on Path group of `w` in `doc` to the point nearest
/// `p` ([`Spine::nearest`]: past the ends of an open path, along the lines of its end segments),
/// staying near `from` when the pointer is following the path from there. The line from the last
/// object's attachment point through the widget counts as the path too, as far as the pointer is
/// ahead of that object along it, so a widget beyond a corner measures from where it is. The flag
/// says whether the point is on that line, where the distance runs on past the end of a closed path
/// instead of round it.
fn length_near(doc: &Document, w: &SpaceWidget, p: Point, from: Option<f64>) -> Option<(f64, bool)> {
    let n = doc.node(w.group)?;
    let rec = n.extras().objects_on_path.as_deref()?;
    let path = attachable_path(n.children()?.iter().find(|c| c.id == rec.path)?)?;
    let spine = Spine::new(path)?;
    let bias = |s: f64| from.filter(|f| f.is_finite()).map_or(0.0, |f| spine.apart(s, f) / 2.0);
    let ahead = (p - w.last).dot(w.dir);
    let line = (ahead >= 0.0).then(|| {
        let s = w.last_length + ahead;
        ((p - w.last).cross(w.dir).abs() + bias(s), s)
    });
    let (s, on_line) = match (spine.nearest(p, from), line) {
        (Some(on), Some(line)) if line.0 < on.0 => (line.1, true),
        (Some(on), _) => (on.1, false),
        (None, line) => (line?.1, true),
    };
    s.is_finite().then_some((s, on_line))
}

/// Dragging the Space widget: the distance the pointer moves along the path moves the last object
/// along it as far, and the objects between follow evenly. With Alt the first object moves the
/// other way as far, so the objects spread from both ends.
#[derive(Clone, Debug)]
pub struct SpaceDrag {
    widget: SpaceWidget,
    start: Point,
    began: bool,
    /// The distance along the path to the point nearest the pointer at the last drag (points).
    last: f64,
    /// The distance the pointer has moved along the path since the press (points).
    along: f64,
    /// The distance between neighboring objects the drag sets (points).
    gap: f64,
    at: Point,
}

impl SpaceDrag {
    /// The drag has left the click: the distance is changing.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// Start a drag when the press `ev` is on the Space widget.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let widget = SpaceWidget::at_point(cx, ev.pos)?;
        let (last, _) = length_near(cx.doc, &widget, ev.pos, None)?;
        Some(Self { widget, start: ev.pos, began: false, last, along: 0.0, gap: widget.gap, at: ev.pos })
    }

    /// The pointer moved to `p` from `self.at`: add the distance it moved along the path. On the
    /// line through the widget, from a point ahead of the last object, that is how far it moved
    /// along the line; otherwise it takes the shorter way round a closed path.
    fn follow(&mut self, doc: &Document, p: Point) {
        let w = &self.widget;
        let Some((t, on_line)) = length_near(doc, w, p, Some(self.last)) else { return };
        let before = (self.at - w.last).dot(w.dir);
        let step = if on_line && before >= 0.0 {
            (p - w.last).dot(w.dir) - before
        } else if w.closed {
            (t - self.last + w.len / 2.0).rem_euclid(w.len) - w.len / 2.0
        } else {
            t - self.last
        };
        if step.is_finite() {
            self.along += step;
            self.last = t;
        }
    }

    /// The Space and start the drag sets: without `alt` the first object stays and the last one goes
    /// no further than the end of an open path; with `alt` the objects are centered on an open path,
    /// or on the first anchor of a closed one, and spread over the whole path at most. On a closed
    /// path the objects go no more than once around.
    fn spacing_at(&self, alt: bool) -> (f64, f64) {
        let w = &self.widget;
        let k = (w.count - 1) as f64;
        let along = self.along;
        let round = w.len / w.count as f64;
        if alt {
            let most = if w.closed { round } else { w.len / k };
            let gap = (w.gap + 2.0 * along / k).clamp(0.0, most);
            let span = gap * k;
            (gap, if w.closed { (-span / 2.0).rem_euclid(w.len) } else { (w.len - span) / 2.0 })
        } else {
            let most = if w.closed { round } else { (w.len - w.start) / k };
            ((w.gap + along / k).clamp(0.0, most.max(0.0)), w.start)
        }
    }

    /// The pointer moved to `p`; `alt` spreads the objects from both ends.
    pub fn drag(&mut self, cx: &ToolContext, p: Point, alt: bool) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Objects on Path Options".into()));
        }
        self.follow(cx.doc, p);
        let (gap, start) = self.spacing_at(alt);
        self.gap = gap;
        self.at = p;
        // Where the objects stay as they are drawn, the group keeps its saved values: Auto, or a
        // Space value the path is too short for.
        let w = &self.widget;
        let (spacing, start) = if (gap - w.gap).abs() < 1e-9 && (start - w.start).abs() < 1e-9 {
            (w.saved.0.map_or_else(|| json!("auto"), |g| json!(g)), json!(w.saved.1))
        } else {
            (json!(gap), json!(start))
        };
        out.push(Action::Preview("object.objectsOnPath.options".into(), json!({ "ids": [w.group.0], "spacing": spacing, "start": start })));
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    /// The distance between the objects next to the pointer, in the document's units.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.began || !cx.measurement_labels {
            return vec![];
        }
        vec![Overlay::Measure { p: self.at, text: cx.len(self.gap) }]
    }
}

/// The Move All widget of a selected Objects on Path group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveAllWidget {
    pub group: NodeId,
    /// The first object's attachment point, in the document.
    pub first: Point,
    /// The widget's center in the document.
    pub at: Point,
    /// The length of the path (points).
    pub len: f64,
    /// Is the path closed?
    pub closed: bool,
    /// Where the first object attaches along the path (points from its first anchor), as drawn.
    pub start: f64,
    /// The farthest along an open path the first object can attach with the others still on it
    /// (points); the path's length on a closed path.
    pub most: f64,
    /// The Space value a drag sets with the start: the distance between the objects as drawn, when
    /// the saved value would draw them apart differently from another start (Auto, or a Space value
    /// an open path is too short for). `None` keeps the saved value.
    pub gap: Option<f64>,
    /// The group's start, as saved.
    pub saved: f64,
}

impl MoveAllWidget {
    /// The widget of the selection at `zoom` (screen px per document point): only when the
    /// selection is one editable Objects on Path group with at least one object on a path with
    /// length, no anchors are Direct-Selected, and the objects have room to move: on an open path,
    /// the distance from the first object to the last is shorter than the path. It sits beyond the
    /// first object's bounds, on the side its bottom faces.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64) -> Option<Self> {
        let (group, rec, children, path) = one_group(doc, selection, zoom)?;
        let count = rec.members.len();
        let first = rec.members.first()?;
        let (len, closed) = path_length(path)?;
        let (start, gap) = layout(len, closed, count, rec.spacing, rec.start);
        let most = if closed { len } else { len - gap * count.saturating_sub(1) as f64 };
        if !closed && (!most.is_finite() || most <= 1e-9) {
            return None;
        }
        // On an open path, Auto and a Space value the path is too short for draw the objects apart
        // by what is left of the path after the start, so another start would change the distance.
        let keeps = closed || count < 2 || rec.spacing.is_some_and(|g| (g - gap).abs() < 1e-9);
        let (at_first, _) = stop(path, count, rec.spacing, rec.start, 0)?;
        let at = beside(doc, children, first, at_first, -top(first), MOVE_ALL_OFFSET_PX / zoom)?;
        let finite = [len, start, most, rec.start].iter().all(|v| v.is_finite());
        finite.then_some(Self { group, first: at_first, at, len, closed, start, most, gap: (!keeps).then_some(gap), saved: rec.start })
    }

    /// The widget of `cx` when `p` is within reach of it.
    fn at_point(cx: &ToolContext, p: Point) -> Option<Self> {
        Self::of(cx.doc, cx.selection, cx.zoom).filter(|w| w.at.distance(p) <= cx.tol(MOVE_ALL_HIT_PX))
    }
}

/// Is `p` over the Move All widget?
pub fn over_move_all(cx: &ToolContext, p: Point) -> bool {
    MoveAllWidget::at_point(cx, p).is_some()
}

/// Dragging the Move All widget: the distance the pointer moves along the path moves every object
/// along it as far, keeping the distances between them. On an open path the objects stop at its
/// ends, and on a closed path they go on round across its first anchor.
#[derive(Clone, Debug)]
pub struct MoveAllDrag {
    widget: MoveAllWidget,
    start: Point,
    began: bool,
    /// The distance along the path to the point nearest the pointer at the last drag (points).
    last: f64,
    /// Where the drag puts the first object along the path (points from its first anchor).
    to: f64,
    at: Point,
}

impl MoveAllDrag {
    /// The drag has left the click: the objects are moving.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// Start a drag when the press `ev` is on the Move All widget.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let widget = MoveAllWidget::at_point(cx, ev.pos)?;
        let (_, last) = group_spine(cx.doc, widget.group)?.nearest(ev.pos, Some(widget.start))?;
        last.is_finite().then_some(Self { widget, start: ev.pos, began: false, last, to: widget.start, at: ev.pos })
    }

    /// The pointer moved to `p`: move the first object as far along the path as the point nearest
    /// the pointer moved, the shorter way round a closed path. On an open path it stays between
    /// the first anchor and [`MoveAllWidget::most`], so moving back from beyond an end moves the
    /// objects back at once.
    fn follow(&mut self, doc: &Document, p: Point) {
        let w = &self.widget;
        let Some((_, t)) = group_spine(doc, w.group).and_then(|s| s.nearest(p, Some(self.last))) else { return };
        let step = if w.closed { (t - self.last + w.len / 2.0).rem_euclid(w.len) - w.len / 2.0 } else { t - self.last };
        if !t.is_finite() || !step.is_finite() {
            return;
        }
        self.last = t;
        self.to = if w.closed {
            let s = (self.to + step).rem_euclid(w.len);
            if s.is_finite() && s < w.len { s } else { 0.0 }
        } else {
            (self.to + step).clamp(0.0, w.most)
        };
    }

    /// The pointer moved to `p`.
    pub fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Objects on Path Options".into()));
        }
        self.follow(cx.doc, p);
        self.at = p;
        let w = &self.widget;
        // Where the objects stay as they are drawn, the group keeps its saved values.
        let params = if (self.to - w.start).abs() < 1e-9 {
            json!({ "ids": [w.group.0], "start": w.saved })
        } else if let Some(gap) = w.gap {
            json!({ "ids": [w.group.0], "start": self.to, "spacing": gap })
        } else {
            json!({ "ids": [w.group.0], "start": self.to })
        };
        out.push(Action::Preview("object.objectsOnPath.options".into(), params));
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    /// The first object's distance along the path next to the pointer, in the document's units.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.began || !cx.measurement_labels {
            return vec![];
        }
        vec![Overlay::Measure { p: self.at, text: cx.len(self.to) }]
    }
}

/// The Select/Move widget of one object of a selected Objects on Path group: a small ring on the
/// object's attachment point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemberWidget {
    pub group: NodeId,
    /// The object.
    pub member: NodeId,
    /// The object's place along the path (0: the first).
    pub index: usize,
    /// The object's attachment point, in the document: the widget's center.
    pub at: Point,
    /// Is the object the chosen one ([`Selection::on_path_member`])?
    pub chosen: bool,
}

impl MemberWidget {
    /// The widgets of the selection at `zoom` (screen px per document point), one per object in
    /// its order along the path that is neither hidden nor locked: only when the selection is one
    /// editable Objects on Path group on a path with length, and no anchors are Direct-Selected.
    pub fn all(doc: &Document, selection: &Selection, zoom: f64) -> Vec<Self> {
        let Some((group, rec, _, path)) = one_group(doc, selection, zoom) else { return vec![] };
        let Some(at) = stops(path, rec.members.len(), rec.spacing, rec.start) else { return vec![] };
        let editable = doc.editable();
        rec.members
            .iter()
            .zip(at)
            .enumerate()
            .filter(|(_, (m, (p, _)))| p.x.is_finite() && p.y.is_finite() && editable(m.id))
            .map(|(index, (m, (at, _)))| Self { group, member: m.id, index, at, chosen: selection.on_path_member == Some(m.id) })
            .collect()
    }

    /// The widget of `cx` nearest `p`, when `p` is within reach of it; of widgets as near, the
    /// chosen one, else the first along the path.
    fn at_point(cx: &ToolContext, p: Point) -> Option<Self> {
        let reach = cx.tol(MEMBER_HIT_PX);
        // Every attachment point lies on the path: a pointer beyond reach of its bounds is over none.
        let (_, _, _, path) = one_group(cx.doc, cx.selection, cx.zoom)?;
        if !path.control_bounds()?.inflate(reach, reach).contains(p) {
            return None;
        }
        let mut best: Option<(f64, Self)> = None;
        for w in Self::all(cx.doc, cx.selection, cx.zoom) {
            let d = w.at.distance(p);
            let nearer = best.is_none_or(|(b, bw)| d < b - 1e-9 || (d <= b + 1e-9 && w.chosen && !bw.chosen));
            if d <= reach && nearer {
                best = Some((d, w));
            }
        }
        best.map(|(_, w)| w)
    }
}

/// Is `p` over a Select/Move widget?
pub fn over_member(cx: &ToolContext, p: Point) -> bool {
    MemberWidget::at_point(cx, p).is_some()
}

/// The highlight of the Select/Move widget under `p`, if any: a ring around it.
pub fn member_hover(cx: &ToolContext, p: Point) -> Option<Overlay> {
    let w = MemberWidget::at_point(cx, p)?;
    let r = cx.tol(f64::from(MEMBER_RADIUS_PX) + 2.0);
    let ring = vectorcraft_geom::shapes::ellipse(vectorcraft_geom::Rect::new(w.at.x - r, w.at.y - r, w.at.x + r, w.at.y + r));
    Some(Overlay::Path { path: ring.to_bezpath(), color: HIGHLIGHT, width: 1.5, dashed: false })
}

/// Dragging a Select/Move widget: the object goes to the place along the path nearest the
/// pointer, and the objects between it and its place move one place toward where it was, except
/// hidden and locked ones (`object.objectsOnPath.moveMember`). On an open path a pointer beyond
/// an end takes the place at that end; on a closed path the pointer goes on round across the first
/// anchor. Where two places are as near, the one the drag is at stays. A click without a drag, and
/// the release of a drag, choose the object (`object.objectsOnPath.selectMember`).
#[derive(Clone, Debug)]
pub struct MemberDrag {
    widget: MemberWidget,
    /// The objects in their order along the path when the press began.
    order: Vec<NodeId>,
    /// The distance along the path to each place (points), in that order.
    places: Vec<f64>,
    /// Can the drag take each place? Not the place of a hidden or locked object.
    open: Vec<bool>,
    /// The length of the path (points).
    len: f64,
    /// Is the path closed?
    closed: bool,
    start: Point,
    began: bool,
    /// The distance along the path to the point nearest the pointer at the last drag (points).
    last: f64,
    /// The place the drag puts the object at.
    target: usize,
}

impl MemberDrag {
    /// The drag has left the click: the object is moving.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// Start a drag when the press `ev` is on a Select/Move widget.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let widget = MemberWidget::at_point(cx, ev.pos)?;
        let n = cx.doc.node(widget.group)?;
        let rec = n.extras().objects_on_path.as_deref()?;
        let (len, closed) = path_length(attachable_path(n.children()?.iter().find(|c| c.id == rec.path)?)?)?;
        let count = rec.members.len();
        let (s0, gap) = layout(len, closed, count, rec.spacing, rec.start);
        let places: Vec<f64> = (0..count).map(|i| nth_length(len, closed, s0, gap, i)).collect();
        let last = *places.get(widget.index)?;
        let editable = cx.doc.editable();
        let open = rec.members.iter().map(|m| editable(m.id)).collect();
        let order = rec.members.iter().map(|m| m.id).collect();
        Some(Self { widget, order, places, open, len, closed, start: ev.pos, began: false, last, target: widget.index })
    }

    /// How far apart along the path the distances `a` and `b` are: the shorter way round a closed
    /// path.
    fn apart(&self, a: f64, b: f64) -> f64 {
        let d = (a - b).abs();
        if self.closed { d.rem_euclid(self.len).min(self.len - d.rem_euclid(self.len)) } else { d }
    }

    /// The place nearest the distance `t` along the path, of the places the drag can take; the
    /// current one where another is as near.
    fn nearest_place(&self, t: f64) -> usize {
        let t = if self.closed { t } else { t.clamp(0.0, self.len) };
        let now = self.places.get(self.target).map_or(f64::INFINITY, |s| self.apart(t, *s));
        let mut best = (self.target, now);
        for (i, s) in self.places.iter().enumerate().filter(|(i, _)| self.open.get(*i).copied().unwrap_or(false)) {
            let d = self.apart(t, *s);
            if d.is_finite() && d < best.1 - 1e-9 {
                best = (i, d);
            }
        }
        best.0
    }

    /// The pointer moved to `p`.
    pub fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Move Object on Path".into()));
        }
        let w = &self.widget;
        let Some((_, t)) = group_spine(cx.doc, w.group).and_then(|s| s.nearest(p, Some(self.last))) else { return out };
        if !t.is_finite() {
            return out;
        }
        self.last = t;
        let target = self.nearest_place(t);
        if target != self.target {
            self.target = target;
            let to = self.order.get(target).copied().unwrap_or(w.member);
            out.push(Action::Preview("object.objectsOnPath.moveMember".into(), json!({ "group": w.group.0, "member": w.member.0, "target": to.0 })));
        }
        out
    }

    /// The release: a drag that moved the object keeps it as one undo step, one that ends where it
    /// began keeps nothing, and either way the object is chosen.
    pub fn finish(self) -> Vec<Action> {
        let choose = Action::Exec("object.objectsOnPath.selectMember".into(), json!({ "member": self.widget.member.0 }));
        match (self.began, self.target == self.widget.index) {
            (false, _) => vec![choose],
            (true, true) => vec![Action::Cancel, choose],
            (true, false) => vec![Action::Commit, choose],
        }
    }
}

/// The path of the Objects on Path group `g` of `doc`, to measure along.
fn group_spine(doc: &Document, g: NodeId) -> Option<Spine> {
    let n = doc.node(g)?;
    let rec = n.extras().objects_on_path.as_deref()?;
    Spine::new(attachable_path(n.children()?.iter().find(|c| c.id == rec.path)?)?)
}

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

pub struct ObjectsOnPathTool {
    /// The path under the pointer the selection can attach to, and its outline.
    hover: Option<(NodeId, BezPath)>,
    /// Where the press was, while the button is down.
    press: Option<Point>,
    /// The Rotate All widget drag, while the button is down after a press on the widget.
    rotating: Option<RotateAllDrag>,
    /// The Space widget drag, while the button is down after a press on the widget.
    spacing: Option<SpaceDrag>,
    /// The Move All widget drag, while the button is down after a press on the widget.
    moving: Option<MoveAllDrag>,
    /// The Select/Move widget drag, while the button is down after a press on a widget.
    member: Option<MemberDrag>,
    /// Where the pointer is while no button is down, for the Select/Move widget it is over.
    pointer: Option<Point>,
    /// The reference point of each object attached next (0-8, row by row from the top left).
    pivot: u8,
    /// The angle of each object attached next, in degrees counter-clockwise from upright on the path.
    rotate: f64,
}

impl Default for ObjectsOnPathTool {
    fn default() -> Self {
        Self { hover: None, press: None, rotating: None, spacing: None, moving: None, member: None, pointer: None, pivot: CENTER, rotate: 0.0 }
    }
}

impl Tool for ObjectsOnPathTool {
    fn id(&self) -> &'static str {
        "objectsOnPath"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        if let Some(d) = &mut self.rotating {
            return match ev.kind {
                PointerKind::Drag => d.drag(cx, p, ev.mods.shift),
                PointerKind::Up => self.rotating.take().map(RotateAllDrag::finish).unwrap_or_default(),
                _ => vec![],
            };
        }
        if let Some(d) = &mut self.spacing {
            return match ev.kind {
                PointerKind::Drag => d.drag(cx, p, ev.mods.alt),
                PointerKind::Up => self.spacing.take().map(SpaceDrag::finish).unwrap_or_default(),
                _ => vec![],
            };
        }
        if let Some(d) = &mut self.moving {
            return match ev.kind {
                PointerKind::Drag => d.drag(cx, p),
                PointerKind::Up => self.moving.take().map(MoveAllDrag::finish).unwrap_or_default(),
                _ => vec![],
            };
        }
        if let Some(d) = &mut self.member {
            return match ev.kind {
                PointerKind::Drag => d.drag(cx, p),
                PointerKind::Up => self.member.take().map(MemberDrag::finish).unwrap_or_default(),
                _ => vec![],
            };
        }
        self.pointer = (ev.kind == PointerKind::Move).then_some(p);
        match ev.kind {
            PointerKind::Move => {
                let widget = over_rotate_all(cx, p) || over_space(cx, p) || over_move_all(cx, p) || over_member(cx, p);
                self.hover = if widget { None } else { target(cx, p) };
                vec![]
            }
            PointerKind::Down => {
                if let Some(d) = RotateAllDrag::hit(cx, ev) {
                    self.rotating = Some(d);
                    self.hover = None;
                    return vec![];
                }
                if let Some(d) = SpaceDrag::hit(cx, ev) {
                    self.spacing = Some(d);
                    self.hover = None;
                    return vec![];
                }
                if let Some(d) = MoveAllDrag::hit(cx, ev) {
                    self.moving = Some(d);
                    self.hover = None;
                    return vec![];
                }
                if let Some(d) = MemberDrag::hit(cx, ev) {
                    self.member = Some(d);
                    self.hover = None;
                    return vec![];
                }
                self.press = Some(p);
                vec![]
            }
            PointerKind::Up => {
                // A press that moved away is not a click.
                let Some(start) = self.press.take().filter(|q| q.distance(p) < cx.tol(3.0)) else { return vec![] };
                let Some((path, _)) = target(cx, start) else { return vec![] };
                self.hover = None;
                vec![Action::Exec("object.objectsOnPath.attach".into(), json!({ "path": path.0, "pivot": self.pivot, "rotate": self.rotate }))]
            }
            _ => vec![],
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o: Vec<Overlay> =
            self.hover.iter().map(|(_, path)| Overlay::Path { path: path.clone(), color: HIGHLIGHT, width: 2.0, dashed: false }).collect();
        if let Some(d) = &self.rotating {
            o.extend(d.overlays(cx));
        }
        if let Some(d) = &self.spacing {
            o.extend(d.overlays(cx));
        }
        if let Some(d) = &self.moving {
            o.extend(d.overlays(cx));
        }
        if let Some(p) = self.pointer.filter(|_| !self.busy()) {
            o.extend(member_hover(cx, p));
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if self.member.is_some() {
            return Cursor::Move;
        }
        let idle = self.spacing.is_none() && self.moving.is_none();
        if self.rotating.is_some() || (idle && over_rotate_all(cx, p)) {
            Cursor::Rotate
        } else if self.spacing.is_some() || (self.moving.is_none() && over_space(cx, p)) {
            Cursor::ArrowHollow
        } else if self.moving.is_some() || over_move_all(cx, p) || over_member(cx, p) {
            Cursor::Move
        } else if target(cx, p).is_some() {
            Cursor::ObjectsOnPath
        } else {
            Cursor::Arrow
        }
    }

    /// Escape while the button is down cancels the click, so the release attaches nothing, or the
    /// Rotate All, Space, Move All or Select/Move drag, so the objects go back to the Rotate, Space,
    /// place or order they had.
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.press = None;
            self.hover = None;
            let rotated = self.rotating.take().is_some_and(|d| d.adjusting());
            let spaced = self.spacing.take().is_some_and(|d| d.adjusting());
            let moved = self.moving.take().is_some_and(|d| d.adjusting());
            let ordered = self.member.take().is_some_and(|d| d.adjusting());
            if rotated || spaced || moved || ordered {
                return vec![Action::Cancel];
            }
        }
        vec![]
    }

    fn options(&self) -> Value {
        json!({ "pivot": self.pivot, "rotate": self.rotate })
    }

    /// `pivot` takes a whole number from 0 to 8 and `rotate` a finite number, clamped to -360 to
    /// 360; other values are ignored.
    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "pivot" => {
                if let Some(n) = value.as_u64().filter(|n| *n <= 8) {
                    self.pivot = n as u8;
                }
            }
            "rotate" => {
                if let Some(r) = value.as_f64().filter(|r| r.is_finite()) {
                    self.rotate = r.clamp(-360.0, 360.0);
                }
            }
            _ => {}
        }
    }

    fn busy(&self) -> bool {
        self.press.is_some() || self.rotating.is_some() || self.spacing.is_some() || self.moving.is_some() || self.member.is_some()
    }

    fn transforming(&self) -> bool {
        self.rotating.as_ref().is_some_and(RotateAllDrag::adjusting)
            || self.spacing.as_ref().is_some_and(SpaceDrag::adjusting)
            || self.moving.as_ref().is_some_and(MoveAllDrag::adjusting)
            || self.member.as_ref().is_some_and(MemberDrag::adjusting)
    }
}

pub fn create(id: &str) -> Option<Box<dyn crate::Tool>> {
    (id == "objectsOnPath").then(|| Box::new(ObjectsOnPathTool::default()) as Box<dyn crate::Tool>)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_doc::{Appearance, Node, NodeKind};
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
    fn the_click_sends_the_tool_options_pivot_and_rotate() {
        let (d, _, square) = doc();
        let mut sel = Selection::default();
        sel.set([square]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        assert_eq!(t.options(), json!({ "pivot": 4, "rotate": 0.0 }), "the center, upright");
        t.set_option("pivot", &json!(0));
        t.set_option("rotate", &json!(45.5));
        // Values out of range are ignored.
        t.set_option("pivot", &json!(9));
        t.set_option("rotate", &json!("x"));
        assert_eq!(t.options(), json!({ "pivot": 0, "rotate": 45.5 }));
        // Rotate is kept within the dialog's -360 to 360.
        t.set_option("rotate", &json!(450));
        assert_eq!(t.options()["rotate"], 360.0);
        t.set_option("rotate", &json!(-720.5));
        assert_eq!(t.options()["rotate"], -360.0);
        t.set_option("rotate", &json!(45.5));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 300.0));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 200.0, 300.0));
        assert!(matches!(acts.as_slice(), [Action::Exec(_, p)] if p["pivot"] == 0 && p["rotate"] == 45.5), "{acts:?}");
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

    /// A document holding an Objects on Path group (the line from (0, 300) to (400, 300), with two
    /// 20 × 20 squares attached by their centers, the first on (0, 300)) whose Rotate is `rotate`,
    /// and a selection of the group.
    fn group_doc(rotate: f64) -> (Document, NodeId, Selection) {
        group_doc_framed(rotate, None)
    }

    /// [`group_doc`] with every object drawn in `frame` (mapping its square onto itself) before it
    /// is placed on the path.
    fn group_doc_framed(rotate: f64, frame: Option<vectorcraft_geom::Affine>) -> (Document, NodeId, Selection) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (line, a, b, g) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
        let line = Node::path(line, shapes::line(Point::new(0.0, 300.0), Point::new(400.0, 300.0)), Appearance::default_art());
        let a = Node::path(a, shapes::rectangle(Rect::new(50.0, 50.0, 70.0, 70.0)), Appearance::default_art());
        let b = Node::path(b, shapes::rectangle(Rect::new(100.0, 50.0, 120.0, 70.0)), Appearance::default_art());
        let mut rec = vectorcraft_doc::objects_on_path::ObjectsOnPath::new(line.id, &[&a, &b], CENTER, rotate);
        if let Some(f) = frame {
            for m in &mut rec.members {
                m.frame = f;
            }
        }
        let mut group = Node::new(g, NodeKind::Group { children: vec![Arc::new(line), Arc::new(a), Arc::new(b)], clip: false });
        group.extras_mut().objects_on_path = Some(Box::new(rec));
        vectorcraft_doc::objects_on_path::place(&mut group, &Document::new(1.0, 1.0).symbol_boxes());
        d.insert(Some(l), 0, group).unwrap();
        let mut sel = Selection::default();
        sel.set([g]);
        (d, g, sel)
    }

    /// The pointer `deg` degrees counter-clockwise on the page from straight above the first
    /// object's attachment point (0, 300), 26 points from it.
    fn around(deg: f64) -> Point {
        let (sin, cos) = deg.to_radians().sin_cos();
        Point::new(-26.0 * sin, 300.0 - 26.0 * cos)
    }

    fn near(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-9
    }

    /// The Rotate a drag's last action previews.
    fn previewed(acts: &[Action]) -> Option<f64> {
        match acts.last() {
            Some(Action::Preview(cmd, p)) if cmd == "object.objectsOnPath.options" => p["rotate"].as_f64(),
            _ => None,
        }
    }

    #[test]
    fn the_rotate_all_widget_sits_above_the_first_object() {
        let (d, g, sel) = group_doc(0.0);
        let w = RotateAllWidget::of(&d, &sel, 1.0).unwrap();
        // The first square spans 290 to 310 down the page: the widget is 16 px above its top.
        assert_eq!(w.group, g);
        assert!(near(w.pivot, Point::new(0.0, 300.0)) && near(w.at, Point::new(0.0, 274.0)), "{w:?}");
        assert!(near(RotateAllWidget::of(&d, &sel, 2.0).unwrap().at, Point::new(0.0, 282.0)));
        // Turned by Rotate, the widget goes round with the object's top.
        let (d, _, sel) = group_doc(90.0);
        let w = RotateAllWidget::of(&d, &sel, 1.0).unwrap();
        assert!(near(w.at, Point::new(-26.0, 300.0)), "{w:?}");
        // Drawn upside down (reflected across the line through their centers), the objects have
        // their tops facing down the page, and the widget goes below the first one.
        let flip = vectorcraft_geom::Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 120.0]);
        let (df, _, sf) = group_doc_framed(0.0, Some(flip));
        let wf = RotateAllWidget::of(&df, &sf, 1.0).unwrap();
        assert!(near(wf.pivot, Point::new(0.0, 300.0)) && near(wf.at, Point::new(0.0, 326.0)), "{wf:?}");
        // No widget for nothing selected, for an object in the group, or for two objects.
        assert!(RotateAllWidget::of(&d, &Selection::default(), 1.0).is_none());
        let children: Vec<NodeId> = d.node(sel.objects[0]).and_then(|n| n.children()).unwrap().iter().map(|c| c.id).collect();
        let mut one = Selection::default();
        one.set([children[1]]);
        assert!(RotateAllWidget::of(&d, &one, 1.0).is_none());
        let (d2, line, square) = doc();
        let mut two = Selection::default();
        two.set([line, square]);
        assert!(RotateAllWidget::of(&d2, &two, 1.0).is_none());
        // Nor for a group that is locked.
        let (mut d3, g3, sel3) = group_doc(0.0);
        if let Some(n) = d3.node_mut(g3) {
            n.locked = true;
        }
        assert!(RotateAllWidget::of(&d3, &sel3, 1.0).is_none());
    }

    /// Dragging the widget a quarter turn counter-clockwise adds 90° to the group's Rotate, with
    /// the Selection tool and with the Objects on Path tool, previewed while dragging and kept as
    /// one undo step on release.
    #[test]
    fn dragging_the_rotate_all_widget_adds_the_angle_to_rotate() {
        let (d, g, sel) = group_doc(30.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let at = RotateAllWidget::of(&d, &sel, 1.0).unwrap().at;
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            assert_eq!(t.cursor(&c, at, Mods::default()), Cursor::Rotate, "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Down, at.x, at.y)).is_empty(), "{}", t.id());
            // The press is 30° round from straight above the attachment point, as Rotate turns the
            // first object; the pointer goes on to 120°.
            let q = around(120.0);
            let a = t.pointer(&c, &ev(PointerKind::Drag, q.x, q.y));
            assert_eq!(a.first(), Some(&Action::Begin("Objects on Path Options".into())), "{}", t.id());
            assert!(matches!(a.last(), Some(Action::Preview(_, p)) if p["ids"] == json!([g.0])), "{a:?}");
            assert!((previewed(&a).unwrap() - 120.0).abs() < 1e-9, "{a:?}");
            assert!(t.transforming() && t.cursor(&c, q, Mods::default()) == Cursor::Rotate);
            assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "120.0°")), "{}", t.id());
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, q.x, q.y)), vec![Action::Commit], "{}", t.id());
            assert!(!t.busy());
            // A click on the widget that does not move changes nothing.
            assert!(t.pointer(&c, &ev(PointerKind::Down, at.x, at.y)).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Up, at.x, at.y)).is_empty(), "{}", t.id());
        }
    }

    /// Shift turns in 15° steps, and a drag that goes on round the attachment point keeps adding
    /// to the angle past half a turn.
    #[test]
    fn shift_turns_the_objects_in_steps_of_fifteen_degrees() {
        let (d, _, sel) = group_doc(0.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        let at = around(0.0);
        t.pointer(&c, &ev(PointerKind::Down, at.x, at.y));
        let q = around(20.0);
        let shift = PointerEvent { mods: Mods { shift: true, ..Mods::default() }, ..ev(PointerKind::Drag, q.x, q.y) };
        assert!((previewed(&t.pointer(&c, &shift)).unwrap() - 15.0).abs() < 1e-9);
        let q = around(-38.0);
        let shift = PointerEvent { mods: Mods { shift: true, ..Mods::default() }, ..ev(PointerKind::Drag, q.x, q.y) };
        assert!((previewed(&t.pointer(&c, &shift)).unwrap() + 45.0).abs() < 1e-9);
        let q = around(20.0);
        assert!((previewed(&t.pointer(&c, &ev(PointerKind::Drag, q.x, q.y))).unwrap() - 20.0).abs() < 1e-9, "without Shift");
        let mut last = 0.0;
        for deg in [100.0, 180.0, 260.0] {
            let q = around(deg);
            last = previewed(&t.pointer(&c, &ev(PointerKind::Drag, q.x, q.y))).unwrap();
        }
        assert!((last - 260.0).abs() < 1e-9, "{last}");
    }

    /// Escape during a Rotate All drag cancels it: the readout is removed, and the release keeps
    /// nothing.
    #[test]
    fn escape_cancels_a_rotate_all_drag() {
        let (d, _, sel) = group_doc(0.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            let at = around(0.0);
            t.pointer(&c, &ev(PointerKind::Down, at.x, at.y));
            let q = around(60.0);
            assert!(previewed(&t.pointer(&c, &ev(PointerKind::Drag, q.x, q.y))).is_some());
            assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{}", t.id());
            assert!(!t.busy() && !t.transforming(), "{}", t.id());
            assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { .. })), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Drag, at.x, at.y)).is_empty(), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Up, at.x, at.y)).is_empty(), "{}: the release keeps nothing", t.id());
        }
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

    /// [`group_doc`] with the group's Space `spacing` from `start`, its objects placed again.
    fn spaced_doc(spacing: Option<f64>, start: f64) -> (Document, NodeId, Selection) {
        let (mut d, g, sel) = group_doc(0.0);
        let empty = Document::new(1.0, 1.0);
        let symbols = empty.symbol_boxes();
        let n = d.node_mut(g).unwrap();
        let rec = n.extras().objects_on_path.as_deref().cloned().unwrap();
        n.extras_mut().objects_on_path = Some(Box::new(vectorcraft_doc::objects_on_path::ObjectsOnPath { spacing, start, ..rec }));
        vectorcraft_doc::objects_on_path::place(n, &symbols);
        (d, g, sel)
    }

    /// The Space and start a drag's last action previews, to 1/1,000,000 pt.
    fn spaced(acts: &[Action]) -> Option<(f64, f64)> {
        let round = |v: f64| (v * 1e6).round() / 1e6 + 0.0;
        match acts.last() {
            Some(Action::Preview(cmd, p)) if cmd == "object.objectsOnPath.options" => {
                Some((round(p["spacing"].as_f64()?), round(p["start"].as_f64()?)))
            }
            _ => None,
        }
    }

    fn alt(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent { mods: Mods { alt: true, ..Mods::default() }, ..ev(kind, x, y) }
    }

    #[test]
    fn the_space_widget_sits_beyond_the_last_object_ahead_along_the_path() {
        // Auto: the second square sits on the end (400, 300) and spans 390 to 410.
        let (d, g, sel) = group_doc(0.0);
        let w = SpaceWidget::of(&d, &sel, 1.0).unwrap();
        assert_eq!((w.group, w.count, w.closed), (g, 2, false));
        assert!(near(w.last, Point::new(400.0, 300.0)) && near(w.at, Point::new(426.0, 300.0)), "{w:?}");
        assert!((w.len - 400.0).abs() < 1e-9 && w.start == 0.0 && (w.gap - 400.0).abs() < 1e-9, "{w:?}");
        assert!(near(SpaceWidget::of(&d, &sel, 2.0).unwrap().at, Point::new(418.0, 300.0)));
        // With a Space set it follows the last object, and Rotate All follows the first one.
        let (d, _, sel) = spaced_doc(Some(100.0), 50.0);
        let w = SpaceWidget::of(&d, &sel, 1.0).unwrap();
        assert!(near(w.last, Point::new(150.0, 300.0)) && near(w.at, Point::new(176.0, 300.0)), "{w:?}");
        assert!(near(RotateAllWidget::of(&d, &sel, 1.0).unwrap().pivot, Point::new(50.0, 300.0)));
        // No widget for an object in the group, or for a locked group.
        let children: Vec<NodeId> = d.node(sel.objects[0]).and_then(|n| n.children()).unwrap().iter().map(|c| c.id).collect();
        let mut one = Selection::default();
        one.set([children[1]]);
        assert!(SpaceWidget::of(&d, &one, 1.0).is_none());
        let (mut d, g, sel) = group_doc(0.0);
        if let Some(n) = d.node_mut(g) {
            n.locked = true;
        }
        assert!(SpaceWidget::of(&d, &sel, 1.0).is_none());
    }

    /// Dragging the widget 100 pt back along the path brings the last object 100 pt nearer the
    /// first one, which stays; with Alt the first one comes as far the other way. Switching Alt
    /// during the drag works from where the drag began.
    #[test]
    fn dragging_the_space_widget_spaces_the_objects_from_the_first_one() {
        let (d, g, sel) = group_doc(0.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let at = SpaceWidget::of(&d, &sel, 1.0).unwrap().at;
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            let id = t.id();
            assert_eq!(t.cursor(&c, at, Mods::default()), Cursor::ArrowHollow, "{id}");
            assert!(t.pointer(&c, &ev(PointerKind::Down, at.x, at.y)).is_empty(), "{id}");
            let a = t.pointer(&c, &ev(PointerKind::Drag, at.x - 100.0, at.y + 30.0));
            assert_eq!(a.first(), Some(&Action::Begin("Objects on Path Options".into())), "{id}");
            assert!(matches!(a.last(), Some(Action::Preview(_, p)) if p["ids"] == json!([g.0])), "{a:?}");
            assert_eq!(spaced(&a), Some((300.0, 0.0)), "{id}: the first object stays");
            assert!(t.transforming() && t.cursor(&c, at, Mods::default()) == Cursor::ArrowHollow, "{id}");
            let label = c.len(300.0);
            assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if *text == label)), "{id}");
            assert_eq!(spaced(&t.pointer(&c, &alt(PointerKind::Drag, at.x - 100.0, at.y))), Some((200.0, 100.0)), "{id}: from both ends");
            assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, at.x - 100.0, at.y))), Some((300.0, 0.0)), "{id}: Alt let go");
            // No further than the end, where the objects are as drawn and the group stays Auto; and no
            // less than 0.
            let a = t.pointer(&c, &ev(PointerKind::Drag, at.x + 50.0, at.y));
            assert!(matches!(a.last(), Some(Action::Preview(_, p)) if p["spacing"] == "auto" && p["start"] == 0.0), "{id}: {a:?}");
            assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, at.x - 900.0, at.y))), Some((0.0, 0.0)), "{id}");
            assert_eq!(spaced(&t.pointer(&c, &alt(PointerKind::Drag, at.x - 900.0, at.y))), Some((0.0, 200.0)), "{id}");
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, at.x, at.y)), vec![Action::Commit], "{id}");
            assert!(!t.busy(), "{id}");
            // A click on the widget that does not move changes nothing.
            assert!(t.pointer(&c, &ev(PointerKind::Down, at.x, at.y)).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Up, at.x, at.y)).is_empty(), "{id}");
        }
    }

    /// A Space value the path is too short for stays when a drag leaves the objects where they are
    /// drawn, and a drag that moves them sets the drawn value.
    #[test]
    fn a_drag_that_leaves_the_objects_as_drawn_keeps_the_saved_space() {
        let (d, _, sel) = spaced_doc(Some(1000.0), 0.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let w = SpaceWidget::of(&d, &sel, 1.0).unwrap();
        assert_eq!((w.gap, w.saved), (400.0, (Some(1000.0), 0.0)), "drawn to the end of the 400 pt path");
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at.x, w.at.y));
        assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, w.at.x + 50.0, w.at.y))), Some((1000.0, 0.0)));
        assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, w.at.x - 50.0, w.at.y))), Some((350.0, 0.0)));
        assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, w.at.x, w.at.y))), Some((1000.0, 0.0)), "back where it began");
    }

    /// On a closed path the objects go no more than once around, and Alt centers them on the
    /// first anchor.
    #[test]
    fn dragging_the_space_widget_on_a_closed_path_wraps_at_the_first_anchor() {
        let w = SpaceWidget {
            group: NodeId(1),
            last: Point::ZERO,
            last_length: 300.0,
            at: Point::ZERO,
            dir: Vec2::new(1.0, 0.0),
            len: 400.0,
            closed: true,
            count: 4,
            start: 0.0,
            gap: 100.0,
            saved: (None, 0.0),
        };
        let moved = |along| SpaceDrag { widget: w, start: Point::ZERO, began: true, last: 0.0, along, gap: w.gap, at: Point::ZERO };
        assert_eq!(moved(30.0).spacing_at(false), (100.0, 0.0), "a quarter of the way round at most");
        assert_eq!(moved(-60.0).spacing_at(false), (80.0, 0.0));
        assert_eq!(moved(-60.0).spacing_at(true), (60.0, 310.0), "the span of 180 centered on the first anchor");
    }

    /// An Objects on Path group of two 20 × 20 squares on an L: 400 pt to the right from (0, 300),
    /// then 300 pt up, with Space `spacing`; and a selection of the group.
    fn corner_doc(spacing: Option<f64>) -> (Document, Selection) {
        let points = [Point::new(0.0, 300.0), Point::new(400.0, 300.0), Point::new(400.0, 0.0)];
        path_doc(vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&points, false)), spacing)
    }

    /// An Objects on Path group of two 20 × 20 squares on `path`, with Space `spacing`; and a
    /// selection of the group.
    fn path_doc(path: vectorcraft_geom::PathData, spacing: Option<f64>) -> (Document, Selection) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (line, a, b, g) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
        let line = Node::path(line, path, Appearance::default_art());
        let a = Node::path(a, shapes::rectangle(Rect::new(50.0, 50.0, 70.0, 70.0)), Appearance::default_art());
        let b = Node::path(b, shapes::rectangle(Rect::new(100.0, 50.0, 120.0, 70.0)), Appearance::default_art());
        let rec = vectorcraft_doc::objects_on_path::ObjectsOnPath::new(line.id, &[&a, &b], CENTER, 0.0);
        let rec = vectorcraft_doc::objects_on_path::ObjectsOnPath { spacing, ..rec };
        let mut group = Node::new(g, NodeKind::Group { children: vec![Arc::new(line), Arc::new(a), Arc::new(b)], clip: false });
        group.extras_mut().objects_on_path = Some(Box::new(rec));
        vectorcraft_doc::objects_on_path::place(&mut group, &Document::new(1.0, 1.0).symbol_boxes());
        d.insert(Some(l), 0, group).unwrap();
        let mut sel = Selection::default();
        sel.set([g]);
        (d, sel)
    }

    /// On an L-shaped path the drag counts the distance along the path, round the corner.
    #[test]
    fn a_space_drag_follows_the_path_round_a_corner() {
        let (d, sel) = corner_doc(None);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        // The path is 700 pt long; the second object sits on its end (400, 0), and the widget above it.
        let w = SpaceWidget::of(&d, &sel, 1.0).unwrap();
        assert!(near(w.last, Point::new(400.0, 0.0)) && (w.gap - 700.0).abs() < 1e-9, "{w:?}");
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at.x, w.at.y));
        // The press, 26 pt beyond the end, is 726 pt along. Down the upright leg to 500 pt along,
        // then round the corner onto the level one at 300 pt along.
        assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, 400.0, 200.0))), Some((474.0, 0.0)));
        assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, 300.0, 310.0))), Some((274.0, 0.0)));
    }

    /// With the last object just short of the corner, the widget sits past the corner; the drag
    /// measures from the widget, so the objects follow the pointer from the start.
    #[test]
    fn a_space_widget_past_a_corner_measures_from_where_it_is() {
        let (d, sel) = corner_doc(Some(390.0));
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let w = SpaceWidget::of(&d, &sel, 1.0).unwrap();
        assert!(near(w.last, Point::new(390.0, 300.0)) && near(w.at, Point::new(416.0, 300.0)) && w.last_length == 390.0, "{w:?}");
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at.x, w.at.y));
        assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, 396.0, 300.0))), Some((370.0, 0.0)), "20 pt back");
        assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, 386.0, 300.0))), Some((360.0, 0.0)));
    }

    /// On a closed path, a drag far ahead along the line through the widget spreads the objects
    /// apart, however far the pointer goes at once.
    #[test]
    fn a_long_drag_ahead_on_a_closed_path_spreads_the_objects() {
        // The square's outline is 800 pt round, from (100, 100) to the right.
        let (d, sel) = path_doc(shapes::rectangle(Rect::new(100.0, 100.0, 300.0, 300.0)), Some(100.0));
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let w = SpaceWidget::of(&d, &sel, 1.0).unwrap();
        assert!(near(w.last, Point::new(200.0, 100.0)) && near(w.at, Point::new(226.0, 100.0)), "{w:?}");
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at.x, w.at.y));
        assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, 236.0, 100.0))), Some((110.0, 0.0)));
        assert_eq!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, 1000.0, 100.0))), Some((400.0, 0.0)), "half way round at most");
    }

    /// The start a Move All drag's last action previews, to 1/1,000,000 pt, and whether it also
    /// sets a Space value.
    fn moved(acts: &[Action]) -> Option<(f64, Option<f64>)> {
        let round = |v: f64| (v * 1e6).round() / 1e6 + 0.0;
        match acts.last() {
            Some(Action::Preview(cmd, p)) if cmd == "object.objectsOnPath.options" && p.get("rotate").is_none() => {
                Some((round(p["start"].as_f64()?), p.get("spacing").and_then(Value::as_f64).map(round)))
            }
            _ => None,
        }
    }

    /// A document holding an Objects on Path group of one 20 × 20 square on the line from (0, 300)
    /// to (400, 300), and a selection of the group.
    fn single_doc() -> (Document, Selection) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (line, a, g) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let line = Node::path(line, shapes::line(Point::new(0.0, 300.0), Point::new(400.0, 300.0)), Appearance::default_art());
        let a = Node::path(a, shapes::rectangle(Rect::new(50.0, 50.0, 70.0, 70.0)), Appearance::default_art());
        let rec = vectorcraft_doc::objects_on_path::ObjectsOnPath::new(line.id, &[&a], CENTER, 0.0);
        let mut group = Node::new(g, NodeKind::Group { children: vec![Arc::new(line), Arc::new(a)], clip: false });
        group.extras_mut().objects_on_path = Some(Box::new(rec));
        vectorcraft_doc::objects_on_path::place(&mut group, &Document::new(1.0, 1.0).symbol_boxes());
        d.insert(Some(l), 0, group).unwrap();
        let mut sel = Selection::default();
        sel.set([g]);
        (d, sel)
    }

    #[test]
    fn the_move_all_widget_sits_below_the_first_object_when_the_objects_have_room() {
        // Space 100 from 50: the first square spans 290 to 310 down the page, and the widget is
        // 16 px below its bottom. The two objects take 100 pt of the 400 pt path, so the first
        // one can go from 0 to 300 along it.
        let (d, g, sel) = spaced_doc(Some(100.0), 50.0);
        let w = MoveAllWidget::of(&d, &sel, 1.0).unwrap();
        assert_eq!(w.group, g);
        assert!(near(w.first, Point::new(50.0, 300.0)) && near(w.at, Point::new(50.0, 326.0)), "{w:?}");
        assert!(w.start == 50.0 && (w.most - 300.0).abs() < 1e-9 && w.gap.is_none() && !w.closed, "{w:?}");
        assert!(near(MoveAllWidget::of(&d, &sel, 2.0).unwrap().at, Point::new(50.0, 318.0)));
        // Auto spreads the objects from one end of an open path to the other: no room to move.
        let (d, _, sel) = group_doc(0.0);
        assert!(MoveAllWidget::of(&d, &sel, 1.0).is_none(), "Auto");
        // Nor with a Space value that reaches the end.
        let (d, _, sel) = spaced_doc(Some(400.0), 0.0);
        assert!(MoveAllWidget::of(&d, &sel, 1.0).is_none(), "the whole path");
        // On a closed path the objects always have room, Auto included.
        let (d, sel) = path_doc(shapes::rectangle(Rect::new(100.0, 100.0, 300.0, 300.0)), None);
        let w = MoveAllWidget::of(&d, &sel, 1.0).unwrap();
        assert!(w.closed && (w.most - 800.0).abs() < 1e-9 && w.gap.is_none(), "{w:?}");
        // One object on an open path can move along all of it.
        let (d, sel) = single_doc();
        let w = MoveAllWidget::of(&d, &sel, 1.0).unwrap();
        assert!(w.start == 0.0 && (w.most - 400.0).abs() < 1e-9 && w.gap.is_none(), "{w:?}");
        // No widget for an object in the group, or for a locked group.
        let (d, _, sel) = spaced_doc(Some(100.0), 50.0);
        let children: Vec<NodeId> = d.node(sel.objects[0]).and_then(|n| n.children()).unwrap().iter().map(|c| c.id).collect();
        let mut one = Selection::default();
        one.set([children[1]]);
        assert!(MoveAllWidget::of(&d, &one, 1.0).is_none());
        let (mut d, g, sel) = spaced_doc(Some(100.0), 50.0);
        if let Some(n) = d.node_mut(g) {
            n.locked = true;
        }
        assert!(MoveAllWidget::of(&d, &sel, 1.0).is_none());
    }

    /// Dragging the widget 80 pt along the path moves both objects 80 pt, keeping the Space, with
    /// the Selection tool and with the Objects on Path tool, previewed while dragging and kept as
    /// one undo step on release. On an open path the objects stop at its ends, and coming back
    /// moves them back at once.
    #[test]
    fn dragging_the_move_all_widget_moves_every_object_along_the_path() {
        let (d, g, sel) = spaced_doc(Some(100.0), 50.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let at = MoveAllWidget::of(&d, &sel, 1.0).unwrap().at;
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            let id = t.id();
            assert_eq!(t.cursor(&c, at, Mods::default()), Cursor::Move, "{id}");
            assert!(t.pointer(&c, &ev(PointerKind::Down, at.x, at.y)).is_empty(), "{id}");
            let a = t.pointer(&c, &ev(PointerKind::Drag, at.x + 80.0, at.y + 10.0));
            assert_eq!(a.first(), Some(&Action::Begin("Objects on Path Options".into())), "{id}");
            assert!(matches!(a.last(), Some(Action::Preview(_, p)) if p["ids"] == json!([g.0])), "{a:?}");
            assert_eq!(moved(&a), Some((130.0, None)), "{id}: only the start changes");
            assert!(t.transforming() && t.cursor(&c, at, Mods::default()) == Cursor::Move, "{id}");
            let label = c.len(130.0);
            assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if *text == label)), "{id}");
            // Shift and Alt change nothing.
            let mods = Mods { shift: true, alt: true, ..Mods::default() };
            assert_eq!(moved(&t.pointer(&c, &PointerEvent { mods, ..ev(PointerKind::Drag, at.x + 90.0, at.y) })), Some((140.0, None)), "{id}");
            // No further than where the last object reaches the end, and back at once.
            assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, at.x + 900.0, at.y))), Some((300.0, None)), "{id}");
            assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, at.x + 880.0, at.y))), Some((280.0, None)), "{id}");
            assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, at.x - 900.0, at.y))), Some((0.0, None)), "{id}");
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, at.x, at.y)), vec![Action::Commit], "{id}");
            assert!(!t.busy(), "{id}");
            // A click on the widget that does not move changes nothing.
            assert!(t.pointer(&c, &ev(PointerKind::Down, at.x, at.y)).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Up, at.x + 1.0, at.y)).is_empty(), "{id}");
        }
    }

    /// Where the saved Space would draw the objects apart differently from another start (Auto,
    /// or a value the path is too short for), the drag sets the distance as drawn with the start;
    /// back where it began, the group keeps its saved values.
    #[test]
    fn a_move_all_drag_keeps_the_distance_the_objects_are_drawn_at() {
        for (spacing, what) in [(Some(1000.0), "a Space value too large for the path"), (None, "Auto")] {
            let (d, _, sel) = spaced_doc(spacing, 100.0);
            let paint = paint();
            let c = cx(&d, &sel, &paint);
            let w = MoveAllWidget::of(&d, &sel, 1.0).unwrap();
            assert!((w.most - 100.0).abs() < 1e-9 && w.gap == Some(300.0), "{what}: {w:?}");
            let mut t = ObjectsOnPathTool::default();
            t.pointer(&c, &ev(PointerKind::Down, w.at.x, w.at.y));
            assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, w.at.x - 40.0, w.at.y))), Some((60.0, Some(300.0))), "{what}");
            let a = t.pointer(&c, &ev(PointerKind::Drag, w.at.x, w.at.y));
            assert_eq!(moved(&a), Some((100.0, None)), "{what}: back where it began");
        }
    }

    /// On an L-shaped path the drag counts the distance along the path, round the corner.
    #[test]
    fn a_move_all_drag_follows_the_path_round_a_corner() {
        let (d, sel) = corner_doc(Some(100.0));
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let w = MoveAllWidget::of(&d, &sel, 1.0).unwrap();
        assert!(near(w.at, Point::new(0.0, 326.0)) && (w.most - 600.0).abs() < 1e-9, "{w:?}");
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at.x, w.at.y));
        assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, 200.0, 326.0))), Some((200.0, None)));
        // Up the upright leg, 200 pt from the corner.
        assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, 426.0, 100.0))), Some((600.0, None)), "no further than the end");
        assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, 426.0, 200.0))), Some((500.0, None)));
    }

    /// On a closed path the objects go on round across the first anchor, both ways.
    #[test]
    fn a_move_all_drag_on_a_closed_path_goes_round_across_the_first_anchor() {
        // The square's outline is 800 pt round, from (100, 100) to the right.
        let (d, sel) = path_doc(shapes::rectangle(Rect::new(100.0, 100.0, 300.0, 300.0)), Some(100.0));
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let w = MoveAllWidget::of(&d, &sel, 1.0).unwrap();
        assert!(near(w.first, Point::new(100.0, 100.0)) && near(w.at, Point::new(100.0, 126.0)), "{w:?}");
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at.x, w.at.y));
        // Down the left side, back before the first anchor.
        assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, 100.0, 150.0))), Some((776.0, None)));
        // Round the corner onto the top, ahead of it again.
        assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, 130.0, 100.0))), Some((56.0, None)));
    }

    /// One object moves along all of an open path.
    #[test]
    fn a_move_all_drag_moves_a_single_object_to_the_end() {
        let (d, sel) = single_doc();
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let w = MoveAllWidget::of(&d, &sel, 1.0).unwrap();
        let mut t = crate::select::SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at.x, w.at.y));
        assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, w.at.x + 150.0, w.at.y))), Some((150.0, None)));
        assert_eq!(moved(&t.pointer(&c, &ev(PointerKind::Drag, w.at.x + 600.0, w.at.y))), Some((400.0, None)));
    }

    /// Escape during a Move All drag cancels it: the readout is removed, and the release keeps
    /// nothing.
    #[test]
    fn escape_cancels_a_move_all_drag() {
        let (d, _, sel) = spaced_doc(Some(100.0), 50.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let at = MoveAllWidget::of(&d, &sel, 1.0).unwrap().at;
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            t.pointer(&c, &ev(PointerKind::Down, at.x, at.y));
            assert!(moved(&t.pointer(&c, &ev(PointerKind::Drag, at.x + 40.0, at.y))).is_some());
            assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{}", t.id());
            assert!(!t.busy() && !t.transforming(), "{}", t.id());
            assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { .. })), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Drag, at.x, at.y)).is_empty(), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Up, at.x, at.y)).is_empty(), "{}: the release keeps nothing", t.id());
        }
    }

    /// Escape during a Space drag cancels it: the readout is removed, and the release keeps
    /// nothing.
    #[test]
    fn escape_cancels_a_space_drag() {
        let (d, _, sel) = group_doc(0.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let at = SpaceWidget::of(&d, &sel, 1.0).unwrap().at;
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            t.pointer(&c, &ev(PointerKind::Down, at.x, at.y));
            assert!(spaced(&t.pointer(&c, &ev(PointerKind::Drag, at.x - 40.0, at.y))).is_some());
            assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{}", t.id());
            assert!(!t.busy() && !t.transforming(), "{}", t.id());
            assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { .. })), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Drag, at.x, at.y)).is_empty(), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Up, at.x, at.y)).is_empty(), "{}: the release keeps nothing", t.id());
        }
    }

    /// A document holding an Objects on Path group of `n` 20 × 20 squares attached by their centers
    /// to `path`, with the group's Space `spacing`, a selection of the group, and the squares' ids
    /// in their order along the path.
    fn members_doc(path: vectorcraft_geom::PathData, n: usize, spacing: Option<f64>) -> (Document, NodeId, Selection, Vec<NodeId>) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (line, g) = (d.alloc_id(), d.alloc_id());
        let line = Node::path(line, path, Appearance::default_art());
        let squares: Vec<Node> = (0..n)
            .map(|i| {
                let x = 50.0 * i as f64;
                Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 50.0, x + 20.0, 70.0)), Appearance::default_art())
            })
            .collect();
        let ids: Vec<NodeId> = squares.iter().map(|s| s.id).collect();
        let rec = vectorcraft_doc::objects_on_path::ObjectsOnPath {
            spacing,
            ..vectorcraft_doc::objects_on_path::ObjectsOnPath::new(line.id, &squares.iter().collect::<Vec<_>>(), CENTER, 0.0)
        };
        let mut children = vec![Arc::new(line)];
        children.extend(squares.into_iter().map(Arc::new));
        let mut group = Node::new(g, NodeKind::Group { children, clip: false });
        group.extras_mut().objects_on_path = Some(Box::new(rec));
        vectorcraft_doc::objects_on_path::place(&mut group, &Document::new(1.0, 1.0).symbol_boxes());
        d.insert(Some(l), 0, group).unwrap();
        let mut sel = Selection::default();
        sel.set([g]);
        (d, g, sel, ids)
    }

    /// Three squares on the line from (0, 300) to (400, 300): at 0, 200 and 400.
    fn three_doc() -> (Document, NodeId, Selection, Vec<NodeId>) {
        members_doc(shapes::line(Point::new(0.0, 300.0), Point::new(400.0, 300.0)), 3, None)
    }

    /// The `target` a drag's actions preview for `object.objectsOnPath.moveMember`, if any.
    fn previewed_target(acts: &[Action]) -> Option<u64> {
        acts.iter().rev().find_map(|a| match a {
            Action::Preview(cmd, p) if cmd == "object.objectsOnPath.moveMember" => p["target"].as_u64(),
            _ => None,
        })
    }

    fn choose(id: NodeId) -> Action {
        Action::Exec("object.objectsOnPath.selectMember".into(), json!({ "member": id.0 }))
    }

    #[test]
    fn a_select_move_widget_sits_on_each_attachment_point() {
        let (d, g, sel, ids) = three_doc();
        let ws = MemberWidget::all(&d, &sel, 1.0);
        let at: Vec<(NodeId, usize, Point, bool)> = ws.iter().map(|w| (w.member, w.index, w.at, w.chosen)).collect();
        assert!(ws.iter().all(|w| w.group == g));
        assert_eq!(at.len(), 3);
        for (i, x) in [0.0, 200.0, 400.0].into_iter().enumerate() {
            assert!(at[i].0 == ids[i] && at[i].1 == i && near(at[i].2, Point::new(x, 300.0)) && !at[i].3, "{at:?}");
        }
        let mut chosen = sel.clone();
        chosen.on_path_member = Some(ids[1]);
        assert_eq!(MemberWidget::all(&d, &chosen, 1.0).iter().map(|w| w.chosen).collect::<Vec<_>>(), [false, true, false]);
        // None for an object in the group, with anchors Direct-Selected, or for a locked group.
        let mut one = Selection::default();
        one.set([ids[0]]);
        assert!(MemberWidget::all(&d, &one, 1.0).is_empty());
        let mut anchors = sel.clone();
        anchors.anchors.insert(ids[0], [(0, 0)].into());
        assert!(MemberWidget::all(&d, &anchors, 1.0).is_empty());
        let (mut locked, lg, lsel, _) = three_doc();
        locked.node_mut(lg).unwrap().locked = true;
        assert!(MemberWidget::all(&locked, &lsel, 1.0).is_empty());
    }

    /// A click on a widget chooses its object without moving it, with the Selection tool and the
    /// Objects on Path tool; a click on the object's art away from the widget does not.
    #[test]
    fn a_click_on_a_select_move_widget_chooses_its_object() {
        let (d, _, sel, ids) = three_doc();
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            t.pointer(&c, &ev(PointerKind::Move, 202.0, 301.0));
            assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Path { color, .. } if *color == HIGHLIGHT)), "{}: highlighted", t.id());
            assert_eq!(t.cursor(&c, Point::new(202.0, 301.0), Mods::default()), Cursor::Move, "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Down, 202.0, 301.0)).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Drag, 203.0, 301.0)).is_empty(), "{}: within the click", t.id());
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 203.0, 301.0)), vec![choose(ids[1])], "{}", t.id());
            assert!(!t.busy());
            // On the art, 8 px from the attachment point: not the widget.
            t.pointer(&c, &ev(PointerKind::Down, 208.0, 300.0));
            let acts = t.pointer(&c, &ev(PointerKind::Up, 208.0, 300.0));
            assert!(!acts.contains(&choose(ids[1])), "{}: {acts:?}", t.id());
        }
    }

    /// A widget at an end of the path is within reach from beyond the path's bounds too, up to
    /// 6 px away.
    #[test]
    fn a_select_move_widget_at_an_end_is_hit_from_beyond_the_path() {
        let (d, _, sel, _) = three_doc();
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        assert!(over_member(&c, Point::new(-5.0, 300.0)), "before the start");
        assert!(over_member(&c, Point::new(405.0, 300.0)), "after the end");
        assert!(over_member(&c, Point::new(0.0, 305.5)), "beside the line");
        assert!(!over_member(&c, Point::new(0.0, 306.5)), "out of reach");
        assert!(!over_member(&c, Point::new(-6.5, 300.0)), "out of reach before the start");
    }

    /// Dragging the middle object's widget past the last one moves it to the last place; dragging
    /// the last one's beyond the start of the line moves it to the first place. Each drag is one
    /// undo step, and the release chooses the object.
    #[test]
    fn dragging_a_select_move_widget_moves_its_object_to_the_nearest_place() {
        let (d, g, sel, ids) = three_doc();
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            t.pointer(&c, &ev(PointerKind::Down, 200.0, 300.0));
            let a = t.pointer(&c, &ev(PointerKind::Drag, 280.0, 310.0));
            assert_eq!(a, vec![Action::Begin("Move Object on Path".into())], "{}: still nearest its own place", t.id());
            assert!(t.transforming() && t.cursor(&c, Point::new(280.0, 310.0), Mods::default()) == Cursor::Move);
            let a = t.pointer(&c, &ev(PointerKind::Drag, 330.0, 300.0));
            assert!(
                matches!(a.as_slice(), [Action::Preview(cmd, p)] if cmd == "object.objectsOnPath.moveMember" && *p == json!({ "group": g.0, "member": ids[1].0, "target": ids[2].0 })),
                "{}: {a:?}",
                t.id()
            );
            assert!(t.pointer(&c, &ev(PointerKind::Drag, 600.0, 300.0)).is_empty(), "{}: beyond the end, the last place", t.id());
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 600.0, 300.0)), vec![Action::Commit, choose(ids[1])], "{}", t.id());
            assert!(!t.busy() && !t.transforming());
            // The last one, back before the start of the line.
            t.pointer(&c, &ev(PointerKind::Down, 400.0, 300.0));
            let a = t.pointer(&c, &ev(PointerKind::Drag, 250.0, 300.0));
            assert_eq!(previewed_target(&a), Some(ids[1].0), "{}: {a:?}", t.id());
            let a = t.pointer(&c, &ev(PointerKind::Drag, -80.0, 300.0));
            assert_eq!(previewed_target(&a), Some(ids[0].0), "{}: {a:?}", t.id());
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, -80.0, 300.0)), vec![Action::Commit, choose(ids[2])], "{}", t.id());
        }
    }

    /// A drag that comes back to its own place previews the object back there and keeps nothing on
    /// release; Escape cancels a drag.
    #[test]
    fn a_select_move_drag_back_to_its_place_or_escaped_keeps_nothing() {
        let (d, _, sel, ids) = three_doc();
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            t.pointer(&c, &ev(PointerKind::Down, 0.0, 300.0));
            assert_eq!(previewed_target(&t.pointer(&c, &ev(PointerKind::Drag, 190.0, 300.0))), Some(ids[1].0));
            assert_eq!(previewed_target(&t.pointer(&c, &ev(PointerKind::Drag, 20.0, 300.0))), Some(ids[0].0), "{}: back onto itself", t.id());
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 20.0, 300.0)), vec![Action::Cancel, choose(ids[0])], "{}", t.id());
            t.pointer(&c, &ev(PointerKind::Down, 0.0, 300.0));
            assert!(previewed_target(&t.pointer(&c, &ev(PointerKind::Drag, 390.0, 300.0))).is_some());
            assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{}", t.id());
            assert!(!t.busy() && !t.transforming(), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Drag, 0.0, 300.0)).is_empty(), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Up, 0.0, 300.0)).is_empty(), "{}: the release keeps nothing", t.id());
        }
    }

    /// Round a closed path the drag goes on across the first anchor: the first object, dragged back
    /// past it, takes the last place.
    #[test]
    fn a_select_move_drag_goes_round_a_closed_path_across_its_first_anchor() {
        // The square's outline is 400 pt round, from (100, 100) to the right and on clockwise: the
        // four objects sit on its corners.
        let (d, _, sel, ids) = members_doc(shapes::rectangle(Rect::new(100.0, 100.0, 200.0, 200.0)), 4, None);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 100.0, 100.0));
        assert!(previewed_target(&t.pointer(&c, &ev(PointerKind::Drag, 100.0, 130.0))).is_none(), "nearer its own place");
        let a = t.pointer(&c, &ev(PointerKind::Drag, 100.0, 180.0));
        assert_eq!(previewed_target(&a), Some(ids[3].0), "{a:?}");
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 100.0, 180.0)), vec![Action::Commit, choose(ids[0])]);
        // Forward from the last one across the first anchor: the first place.
        t.pointer(&c, &ev(PointerKind::Down, 100.0, 200.0));
        let a = t.pointer(&c, &ev(PointerKind::Drag, 100.0, 120.0));
        assert_eq!(previewed_target(&a), Some(ids[0].0), "{a:?}");
    }

    /// A hidden or locked object has no widget, and a drag does not take its place: the first
    /// object, dragged onto the locked last one's place, stays at the nearest place it can take.
    #[test]
    fn a_locked_object_has_no_select_move_widget_and_keeps_its_place() {
        let (mut d, _, sel, ids) = three_doc();
        d.node_mut(ids[2]).unwrap().locked = true;
        assert_eq!(MemberWidget::all(&d, &sel, 1.0).iter().map(|w| w.member).collect::<Vec<_>>(), ids[..2]);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 400.0, 300.0));
        assert!(t.pointer(&c, &ev(PointerKind::Up, 400.0, 300.0)).iter().all(|a| *a != choose(ids[2])), "no widget to press");
        t.pointer(&c, &ev(PointerKind::Down, 0.0, 300.0));
        assert_eq!(previewed_target(&t.pointer(&c, &ev(PointerKind::Drag, 390.0, 300.0))), Some(ids[1].0));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 390.0, 300.0)), vec![Action::Commit, choose(ids[0])]);
    }

    /// Of widgets on the same point (no Space between the objects), a press takes the chosen
    /// object's, else the first one's; between places as near, a drag keeps the place it is at.
    #[test]
    fn stacked_select_move_widgets_take_the_chosen_one_and_ties_keep_the_place() {
        let (d, _, mut sel, ids) = members_doc(shapes::line(Point::new(0.0, 300.0), Point::new(400.0, 300.0)), 3, Some(0.0));
        let paint = paint();
        {
            let c = cx(&d, &sel, &paint);
            let mut t = ObjectsOnPathTool::default();
            t.pointer(&c, &ev(PointerKind::Down, 0.0, 300.0));
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 0.0, 300.0)), vec![choose(ids[0])]);
            // Every place is as far as the others: the drag stays where it is.
            t.pointer(&c, &ev(PointerKind::Down, 0.0, 300.0));
            assert!(previewed_target(&t.pointer(&c, &ev(PointerKind::Drag, 200.0, 300.0))).is_none());
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 200.0, 300.0)), vec![Action::Cancel, choose(ids[0])]);
        }
        sel.on_path_member = Some(ids[2]);
        let c = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 0.0, 300.0));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 0.0, 300.0)), vec![choose(ids[2])], "the chosen one");
    }
}
