//! Blend widgets: two handles beside the spine of one selected blend, at the point halfway along
//! the spine, one 16 screen pixels above it and one 16 pixels below it. Where other artwork crowds
//! that spot (the boxes of three or more other visible objects hold it), a widget moves further
//! out in 16-pixel steps, up to 144 pixels, to the first spot that is not crowded, and the line
//! the canvas draws from the halfway point to the widget grows with it. The upper one sets the
//! blend's color shift (`colorShift`, -100 to 100) and the lower one its easing strength (`ramp`,
//! 0 to 100). The lower one shows only when the steps follow a spine (a replaced or edited spine,
//! or Align to Path), where easing moves them. A blend made without a spine shows the upper one
//! alone, placed in the same way against the straight lines between its key centers that its steps
//! are laid out along ([`blend_layout_line`]); the canvas does not draw those lines. A drag along
//! the spine's direction at the press changes the value by one per screen pixel, starting from the
//! value at the press, and the arrow
//! keys during the drag change it by one (Shift: by ten); Escape or a switch to another tool
//! cancels the drag and puts the value back. Each drag previews
//! `object.blend.setEasing` and is one undo step, or none when the value ends where it started; a
//! click changes nothing. The Selection and Blend tools hit the widgets and the canvas draws them
//! from the same geometry.

use serde_json::json;
use vectorcraft_doc::live::{Spine, blend_layout_line};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, Selection};
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::{Action, Overlay, PointerEvent, ToolContext};

/// How far (screen px) a widget's center sits above or below the spine's halfway point.
pub const OFFSET_PX: f64 = 16.0;
/// The widget's radius (screen px), as the canvas draws it.
pub const RADIUS_PX: f32 = 4.0;
/// How near (screen px) the pointer must be to hit a widget.
const HIT_PX: f64 = 5.0;
/// Points of value per screen pixel of drag along the spine.
const PER_PX: f64 = 1.0;
/// The undo step a drag records, as the Blend panel's changes do.
const LABEL: &str = "Blend Options";
/// How many [`OFFSET_PX`] steps further out a widget may move from a crowded spot.
const CLEAR_STEPS: usize = 8;
/// A spot is crowded when the boxes of this many other visible objects hold it.
const CROWDED: usize = 3;
/// The most nodes looked at to find what crowds the widgets, topmost first.
const LOOK: usize = 1024;

/// What a blend widget sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendControl {
    /// The upper widget: the color shift.
    Appearance,
    /// The lower widget: the easing strength.
    Easing,
}

impl BlendControl {
    /// The `object.blend.setEasing` param this widget sets.
    fn key(self) -> &'static str {
        match self {
            Self::Appearance => "colorShift",
            Self::Easing => "ramp",
        }
    }

    /// The values the param takes.
    fn range(self) -> (f64, f64) {
        match self {
            Self::Appearance => (-100.0, 100.0),
            Self::Easing => (0.0, 100.0),
        }
    }
}

/// One widget of the selected blend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendWidget {
    pub id: NodeId,
    pub control: BlendControl,
    /// The spine's halfway point, in the document (for a blend without a spine, the halfway point of
    /// the line its steps are laid out along).
    pub on: Point,
    /// The widget's center in the document: [`OFFSET_PX`] from `on`, or further out where other
    /// artwork crowds that spot.
    pub at: Point,
    /// The spine's direction at `on` (unit length, in the document).
    pub along: Vec2,
    /// The value the widget sets, as it is now.
    pub value: f64,
}

impl BlendWidget {
    /// The widgets of the selection at `zoom` (screen px per document point), with `up` the
    /// document direction of the screen's up: the Appearance widget, then the Easing widget. Empty
    /// unless the selection is one whole blend, visible and unlocked, with two or more keys and a
    /// spine (or, without one, a step layout line) of some length, and no anchors are selected.
    /// Each widget sits at the first spot out from the spine, in [`OFFSET_PX`] steps, that the
    /// boxes of fewer than three other visible objects hold, at most nine steps out; where every
    /// spot is crowded it stays at the first.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64, up: Vec2) -> Vec<Self> {
        let up_len = up.hypot();
        let usable = |v: f64| v.is_finite() && v > 0.0;
        if !selection.anchors.is_empty() || selection.symbol_part.is_some() || !usable(zoom) || !usable(up_len) {
            return vec![];
        }
        let [id] = selection.objects.as_slice() else { return vec![] };
        let Some(NodeKind::Blend { children, spec }) = doc.node(*id).map(|n| &n.kind) else { return vec![] };
        if children.len() < 2 || !doc.is_editable(*id) {
            return vec![];
        }
        let Some(spine) = blend_layout_line(children, spec).and_then(|path| Spine::new(&path)) else { return vec![] };
        if !usable(spine.length()) {
            return vec![];
        }
        let (on, angle) = spine.at(0.5);
        let along = Vec2::new(angle.cos(), angle.sin());
        if !(on.x.is_finite() && on.y.is_finite() && along.x.is_finite() && along.y.is_finite()) {
            return vec![];
        }
        let lift = up * (OFFSET_PX / zoom / up_len);
        let reach = lift.hypot() * (CLEAR_STEPS + 1) as f64 + f64::from(RADIUS_PX) / zoom;
        let boxes = boxes_near(doc, *id, Rect::new(on.x - reach, on.y - reach, on.x + reach, on.y + reach));
        let clear = |side: Vec2| {
            let crowded = |p: Point| boxes.iter().filter(|b| b.x0 <= p.x && p.x <= b.x1 && b.y0 <= p.y && p.y <= b.y1).count() >= CROWDED;
            (1..=CLEAR_STEPS + 1).map(|k| on + side * k as f64).find(|p| !crowded(*p)).unwrap_or(on + side)
        };
        let value = |v: f64, control: BlendControl| {
            let (lo, hi) = control.range();
            if v.is_finite() { v.clamp(lo, hi) } else { 0.0 }
        };
        let shift = value(spec.color_shift, BlendControl::Appearance);
        let mut out = vec![Self { id: *id, control: BlendControl::Appearance, on, at: clear(lift), along, value: shift }];
        if spec.eases_along_spine() {
            let ramp = value(spec.ease.ramp * 100.0, BlendControl::Easing);
            out.push(Self { id: *id, control: BlendControl::Easing, on, at: clear(-lift), along, value: ramp });
        }
        out
    }

    /// The widgets the tool in `cx` can drag.
    pub fn for_tool(cx: &ToolContext) -> Vec<Self> {
        Self::of(cx.doc, cx.selection, cx.zoom, screen_up(cx))
    }

    /// The widget of `cx` nearest `p` within reach.
    fn at_point(cx: &ToolContext, p: Point) -> Option<Self> {
        let d = |w: &Self| w.at.distance(p);
        Self::for_tool(cx).into_iter().filter(|w| d(w) <= cx.tol(HIT_PX)).min_by(|a, b| d(a).total_cmp(&d(b)))
    }
}

/// The document direction of the screen's up (the page's up without a window).
fn screen_up(cx: &ToolContext) -> Vec2 {
    cx.screen.map(|s| -s.down).filter(|v| v.x.is_finite() && v.y.is_finite() && v.hypot() > 0.0).unwrap_or(Vec2::new(0.0, -1.0))
}

/// The geometric boxes of the visible objects other than blend `skip` that reach into `region`.
/// The walk goes into visible layers and groups without a clipping path, topmost first, and looks
/// at no more than [`LOOK`] nodes; any other object counts by its own box.
fn boxes_near(doc: &Document, skip: NodeId, region: Rect) -> Vec<Rect> {
    let reaches = |b: &Rect| b.x0 <= region.x1 && region.x0 <= b.x1 && b.y0 <= region.y1 && region.y0 <= b.y1;
    // Each node is queued at most once, so `room` bounds both the queue and the walk; a container
    // with more members than there is room for queues its topmost ones.
    fn queue<'a>(nodes: &'a [std::sync::Arc<Node>], skip: NodeId, room: &mut usize) -> impl Iterator<Item = &'a Node> {
        let take = nodes.len().min(*room);
        *room -= take;
        nodes.iter().skip(nodes.len() - take).filter(move |c| c.visible && c.id != skip).map(|c| &**c)
    }
    let mut room = LOOK;
    let mut stack: Vec<&Node> = queue(&doc.layers, skip, &mut room).collect();
    let mut out = vec![];
    while let Some(n) = stack.pop() {
        match &n.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                stack.extend(queue(children, skip, &mut room));
            }
            _ => out.extend(n.geometric_bounds().filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && reaches(b))),
        }
    }
    out
}

/// Is `p` over a blend widget?
pub fn over_widget(cx: &ToolContext, p: Point) -> bool {
    BlendWidget::at_point(cx, p).is_some()
}

/// Dragging a blend widget: its value follows the pointer along the spine's direction at the
/// press.
#[derive(Clone, Debug)]
pub struct BlendWidgetDrag {
    widget: BlendWidget,
    start: Point,
    /// The pointer's last position.
    at: Point,
    /// What the arrow keys added.
    keys: f64,
    began: bool,
    value: f64,
}

impl BlendWidgetDrag {
    /// The drag has left the click: the value is changing.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// Start a drag when the press `ev` is on a blend widget of the selection.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let widget = BlendWidget::at_point(cx, ev.pos)?;
        Some(Self { widget, start: ev.pos, at: ev.pos, keys: 0.0, began: false, value: widget.value })
    }

    /// The value at the press plus the pointer's move along the spine, before the keys' steps
    /// and the range.
    fn dragged(&self, cx: &ToolContext) -> f64 {
        let moved = (self.at - self.start).dot(self.widget.along) * cx.zoom;
        self.widget.value + if moved.is_finite() { (moved * PER_PX).round() } else { 0.0 }
    }

    /// The value for the pointer at `self.at` and the keys pressed, within the range.
    fn value_now(&self, cx: &ToolContext) -> f64 {
        let (lo, hi) = self.widget.control.range();
        (self.dragged(cx) + self.keys).clamp(lo, hi)
    }

    /// The actions that show the value now: the drag's start when it begins, then a preview.
    fn preview(&mut self, cx: &ToolContext) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            self.began = true;
            out.push(Action::Begin(LABEL.into()));
        }
        self.value = self.value_now(cx);
        let mut params = json!({ "id": self.widget.id.0 });
        params[self.widget.control.key()] = json!(self.value);
        out.push(Action::Preview("object.blend.setEasing".into(), params));
        out
    }

    pub fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        self.at = p;
        if !self.began && p.distance(self.start) < cx.tol(3.0) {
            return vec![];
        }
        self.preview(cx)
    }

    /// An arrow key during the drag: one point more (`more`) or less, ten with Shift.
    pub fn step(&mut self, cx: &ToolContext, more: bool, big: bool) -> Vec<Action> {
        let by = if big { 10.0 } else { 1.0 };
        // The step goes from the value shown, so past either end (by keys or by the drag) the way
        // back starts at once.
        let (lo, hi) = self.widget.control.range();
        let stepped = (self.value_now(cx) + if more { by } else { -by }).clamp(lo, hi);
        self.keys = stepped - self.dragged(cx);
        self.preview(cx)
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    /// The value next to the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.began || !cx.measurement_labels {
            return vec![];
        }
        vec![Overlay::Measure { p: self.at, text: format!("{:.0}%", self.value) }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select::SelectionTool;
    use crate::testutil::*;
    use crate::{Mods, PointerKind, Tool, ToolKey};
    use std::sync::Arc;
    use vectorcraft_doc::live::{BlendEase, BlendEaseType, BlendOrientation, BlendSpec};
    use vectorcraft_doc::{Appearance, Node};
    use vectorcraft_geom::{PathData, Rect, SubPath, shapes};

    const UP: Vec2 = Vec2::new(0.0, -1.0);

    fn square(d: &mut Document, x: f64, y: f64) -> Arc<Node> {
        let id = d.alloc_id();
        Arc::new(Node::path(id, shapes::rectangle(Rect::new(x, y, x + 20.0, y + 20.0)), Appearance::default_art()))
    }

    /// A blend of squares centered at `centers`, with `spec`, selected.
    fn blend_doc(centers: &[(f64, f64)], spec: BlendSpec) -> (Document, NodeId, Selection) {
        let mut d = Document::new(800.0, 800.0);
        let l = d.layers[0].id;
        let children = centers.iter().map(|(x, y)| square(&mut d, x - 10.0, y - 10.0)).collect();
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::new(id, NodeKind::Blend { children, spec: Box::new(spec) })).unwrap();
        let mut s = Selection::default();
        s.add(id);
        (d, id, s)
    }

    /// Steps along a spine (Align to Path), eased In Out at 40%, color shift -20.
    fn eased() -> BlendSpec {
        BlendSpec {
            orientation: BlendOrientation::AlignToPath,
            ease: BlendEase { kind: BlendEaseType::InOut, ramp: 0.4 },
            color_shift: -20.0,
            ..Default::default()
        }
    }

    fn ev(kind: PointerKind, p: Point) -> PointerEvent {
        PointerEvent::new(kind, p.x, p.y)
    }

    fn near(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn the_widgets_sit_above_and_below_the_spines_halfway_point() {
        let (d, id, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        let w = BlendWidget::of(&d, &s, 1.0, UP);
        assert_eq!(w.iter().map(|w| w.control).collect::<Vec<_>>(), [BlendControl::Appearance, BlendControl::Easing]);
        assert!(w.iter().all(|w| w.id == id && near(w.on, Point::new(200.0, 200.0))), "{w:?}");
        assert!(near(w[0].at, Point::new(200.0, 184.0)) && near(w[1].at, Point::new(200.0, 216.0)), "{w:?}");
        assert_eq!((w[0].value, w[1].value), (-20.0, 40.0));
        // 16 screen pixels at any zoom, and above on the screen in a turned view.
        assert!(near(BlendWidget::of(&d, &s, 4.0, UP)[0].at, Point::new(200.0, 196.0)));
        let turned = BlendWidget::of(&d, &s, 1.0, Vec2::new(1.0, 0.0));
        assert!(near(turned[0].at, Point::new(216.0, 200.0)) && near(turned[1].at, Point::new(184.0, 200.0)), "{turned:?}");
        // Halfway along by length: three keys spaced unevenly.
        let (d, _, s) = blend_doc(&[(0.0, 0.0), (100.0, 0.0), (100.0, 300.0)], eased());
        assert!(near(BlendWidget::of(&d, &s, 1.0, UP)[0].on, Point::new(100.0, 100.0)));
    }

    /// Adds a filled rectangle from `(x0, y0)` to `(x1, y1)` on top of the first layer.
    fn cover(d: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64) -> NodeId {
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let at = d.layers[0].children().map_or(0, |c| c.len());
        d.insert(Some(l), at, Node::path(id, shapes::rectangle(Rect::new(x0, y0, x1, y1)), Appearance::default_art())).unwrap();
        id
    }

    #[test]
    fn a_widget_moves_out_from_crowded_artwork_and_its_line_follows() {
        let (mut d, _, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        // Two objects under the Appearance widget's spot do not crowd it.
        for _ in 0..2 {
            cover(&mut d, 190.0, 175.0, 210.0, 195.0);
        }
        assert!(near(BlendWidget::of(&d, &s, 1.0, UP)[0].at, Point::new(200.0, 184.0)));
        // A third does: the widget moves 16 px further out, past them, and the Easing widget below
        // stays where it is. The halfway point it joins stays the same.
        let third = cover(&mut d, 190.0, 175.0, 210.0, 195.0);
        let w = BlendWidget::of(&d, &s, 1.0, UP);
        assert!(near(w[0].at, Point::new(200.0, 168.0)) && near(w[0].on, Point::new(200.0, 200.0)), "{w:?}");
        assert!(near(w[1].at, Point::new(200.0, 216.0)), "{w:?}");
        // A hidden object does not count.
        d.node_mut(third).unwrap().visible = false;
        assert!(near(BlendWidget::of(&d, &s, 1.0, UP)[0].at, Point::new(200.0, 184.0)));
        // The walk looks at the topmost 1024 nodes only: with more objects above them, the covers
        // below are not looked at.
        d.node_mut(third).unwrap().visible = true;
        let mut many = d.clone();
        for i in 0..LOOK {
            let x = (i % 30) as f64 * 25.0;
            cover(&mut many, x, 500.0, x + 10.0, 510.0);
        }
        assert!(near(BlendWidget::of(&many, &s, 1.0, UP)[0].at, Point::new(200.0, 184.0)));
        assert!(near(BlendWidget::of(&d, &s, 1.0, UP)[0].at, Point::new(200.0, 168.0)));
        // Crowded all the way out (past 144 px): the widget stays at the first spot.
        let (mut d, _, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        for _ in 0..3 {
            cover(&mut d, 150.0, 0.0, 250.0, 199.0);
        }
        assert!(near(BlendWidget::of(&d, &s, 1.0, UP)[0].at, Point::new(200.0, 184.0)));
        // The blend's own keys never crowd it: three 100 pt keys that all hold both spots.
        let mut d = Document::new(800.0, 800.0);
        let l = d.layers[0].id;
        let big = |d: &mut Document, x: f64| {
            let id = d.alloc_id();
            Arc::new(Node::path(id, shapes::rectangle(Rect::new(x - 50.0, 150.0, x + 50.0, 250.0)), Appearance::default_art()))
        };
        let children = vec![big(&mut d, 150.0), big(&mut d, 200.0), big(&mut d, 250.0)];
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::new(id, NodeKind::Blend { children, spec: Box::new(eased()) })).unwrap();
        let mut s = Selection::default();
        s.add(id);
        let w = BlendWidget::of(&d, &s, 1.0, UP);
        assert!(near(w[0].at, Point::new(200.0, 184.0)) && near(w[1].at, Point::new(200.0, 216.0)), "{w:?}");
    }

    #[test]
    fn the_easing_widget_needs_steps_that_follow_a_spine() {
        // The straight lines between the keys with Align to Page: the steps do not follow a spine.
        let (d, _, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], BlendSpec::default());
        assert_eq!(BlendWidget::of(&d, &s, 1.0, UP).iter().map(|w| w.control).collect::<Vec<_>>(), [BlendControl::Appearance]);
        // A replaced spine, closed, with the keys spread along it.
        let spine = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(200.0, 200.0)], true));
        let (d, _, s) = blend_doc(&[(0.0, 0.0), (200.0, 0.0)], BlendSpec { spine: Some(spine), ..Default::default() });
        let w = BlendWidget::of(&d, &s, 1.0, UP);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].on.x.is_finite() && w[0].on.y.is_finite());
    }

    #[test]
    fn no_widgets_for_a_selection_they_cannot_edit() {
        let (d, id, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        assert!(BlendWidget::of(&d, &Selection::default(), 1.0, UP).is_empty(), "nothing selected");
        assert!(BlendWidget::of(&d, &s, 0.0, UP).is_empty() && BlendWidget::of(&d, &s, f64::NAN, UP).is_empty(), "no zoom");
        assert!(BlendWidget::of(&d, &s, 1.0, Vec2::ZERO).is_empty(), "no up");
        // Two blends, or a blend and another object.
        let (mut two, a, _) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        let l = two.layers[0].id;
        let keys = vec![square(&mut two, 0.0, 400.0), square(&mut two, 200.0, 400.0)];
        let b = two.alloc_id();
        two.insert(Some(l), 0, Node::new(b, NodeKind::Blend { children: keys, spec: Box::new(eased()) })).unwrap();
        let mut both = Selection::default();
        both.add(a);
        both.add(b);
        assert!(BlendWidget::of(&two, &both, 1.0, UP).is_empty(), "two blends");
        // Locked or hidden.
        for hide in [false, true] {
            let mut d = d.clone();
            let n = d.node_mut(id).unwrap();
            (n.visible, n.locked) = (!hide, !hide);
            assert!(BlendWidget::of(&d, &s, 1.0, UP).is_empty(), "hidden: {hide}");
        }
        // A key object selected on its own is not the whole blend.
        let NodeKind::Blend { children, .. } = &d.node(id).unwrap().kind else { panic!("not a blend") };
        let mut key = Selection::default();
        key.add(children[0].id);
        assert!(BlendWidget::of(&d, &key, 1.0, UP).is_empty(), "a key");
        // Keys on one spot: the spine has no length.
        let (d, _, s) = blend_doc(&[(100.0, 200.0), (100.0, 200.0)], eased());
        assert!(BlendWidget::of(&d, &s, 1.0, UP).is_empty(), "no length");
        // One key.
        let (d, _, s) = blend_doc(&[(100.0, 200.0)], eased());
        assert!(BlendWidget::of(&d, &s, 1.0, UP).is_empty(), "one key");
        // Without a spine: locked or hidden, or keys on one spot.
        let (d, id, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], BlendSpec { no_spine: true, ..eased() });
        for hide in [false, true] {
            let mut d = d.clone();
            let n = d.node_mut(id).unwrap();
            (n.visible, n.locked) = (!hide, !hide);
            assert!(BlendWidget::of(&d, &s, 1.0, UP).is_empty(), "no spine, hidden: {hide}");
        }
        let (d, _, s) = blend_doc(&[(100.0, 200.0), (100.0, 200.0)], BlendSpec { no_spine: true, ..eased() });
        assert!(BlendWidget::of(&d, &s, 1.0, UP).is_empty(), "no spine, no length");
    }

    /// A blend made without a spine (Create Spine › Never), Align to Path included, shows the
    /// Appearance widget alone, at the halfway point by length of the straight lines between its
    /// key centers.
    #[test]
    fn a_blend_without_a_spine_shows_the_appearance_widget_alone() {
        let no_spine = || BlendSpec { no_spine: true, ..eased() };
        let (d, id, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], no_spine());
        let w = BlendWidget::of(&d, &s, 1.0, UP);
        assert_eq!(w.iter().map(|w| w.control).collect::<Vec<_>>(), [BlendControl::Appearance], "{w:?}");
        assert!(w[0].id == id && near(w[0].on, Point::new(200.0, 200.0)) && near(w[0].at, Point::new(200.0, 184.0)), "{w:?}");
        assert_eq!(w[0].value, -20.0);
        // Halfway along by length, not the middle key: (0, 0), (100, 0), (100, 300) is 400 long.
        let (d, _, s) = blend_doc(&[(0.0, 0.0), (100.0, 0.0), (100.0, 300.0)], no_spine());
        let w = BlendWidget::of(&d, &s, 1.0, UP);
        assert!(w.len() == 1 && near(w[0].on, Point::new(100.0, 100.0)), "{w:?}");
        // There the line runs down, so a drag down adds.
        assert!(near(Point::new(w[0].along.x, w[0].along.y), Point::new(0.0, 1.0)), "{w:?}");
        // Two keys on one spot and a third away: the line still has length.
        let (d, _, s) = blend_doc(&[(100.0, 200.0), (100.0, 200.0), (300.0, 200.0)], no_spine());
        let w = BlendWidget::of(&d, &s, 1.0, UP);
        assert!(w.len() == 1 && near(w[0].on, Point::new(200.0, 200.0)), "{w:?}");
    }

    /// With the Selection or the Blend tool, a drag of a spineless blend's Appearance widget
    /// previews the color shift alone, in one undo step.
    #[test]
    fn dragging_the_appearance_widget_of_a_blend_without_a_spine() {
        let (d, id, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], BlendSpec { no_spine: true, ..eased() });
        let p = paint();
        let c = cx(&d, &s, &p);
        let at = Point::new(200.0, 184.0);
        assert!(over_widget(&c, at) && !over_widget(&c, Point::new(200.0, 216.0)), "no Easing widget below");
        let tools: [Box<dyn Tool>; 2] = [Box::new(SelectionTool::default()), Box::new(crate::meshblend::BlendTool::default())];
        for mut t in tools {
            assert!(t.pointer(&c, &ev(PointerKind::Down, at)).is_empty());
            let a = t.pointer(&c, &ev(PointerKind::Drag, at + Vec2::new(30.0, 0.0)));
            assert_eq!(
                a,
                vec![Action::Begin(LABEL.into()), Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "colorShift": 10.0}))]
            );
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, at + Vec2::new(30.0, 0.0))), vec![Action::Commit]);
        }
    }

    /// A drag of the Easing widget along the spine changes the ramp by one per pixel from its
    /// value at the press, in one undo step, without changing the type or the color shift.
    #[test]
    fn dragging_the_easing_widget_sets_the_ramp_from_its_value() {
        let (d, id, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        let p = paint();
        let c = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        let at = Point::new(200.0, 216.0);
        assert!(t.pointer(&c, &ev(PointerKind::Down, at)).is_empty());
        // A small move first: no jump, the value stays at the press's 40.
        assert!(t.pointer(&c, &ev(PointerKind::Drag, at + Vec2::new(1.0, 1.0))).is_empty());
        let a = t.pointer(&c, &ev(PointerKind::Drag, at + Vec2::new(3.0, 30.0)));
        assert_eq!(a[0], Action::Begin(LABEL.into()));
        assert_eq!(a[1], Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "ramp": 43.0})), "across the spine counts for nothing");
        assert!(t.transforming());
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "43%")));
        let a = t.pointer(&c, &ev(PointerKind::Drag, at + Vec2::new(-70.0, 0.0)));
        assert_eq!(a, vec![Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "ramp": 0.0}))], "clamped at 0");
        let a = t.pointer(&c, &ev(PointerKind::Drag, at + Vec2::new(500.0, 0.0)));
        assert_eq!(a, vec![Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "ramp": 100.0}))], "clamped at 100");
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, at)), vec![Action::Commit]);
        // A click that doesn't move sends nothing.
        assert!(t.pointer(&c, &ev(PointerKind::Down, at)).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, at)).is_empty());
    }

    /// The Appearance widget sets the color shift alone, and at 4× zoom a pixel is a quarter point.
    #[test]
    fn dragging_the_appearance_widget_sets_the_color_shift() {
        let (d, id, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        let p = paint();
        let c = ToolContext { zoom: 4.0, ..cx(&d, &s, &p) };
        let mut t = SelectionTool::default();
        let at = Point::new(200.0, 196.0);
        t.pointer(&c, &ev(PointerKind::Down, at));
        let a = t.pointer(&c, &ev(PointerKind::Drag, at + Vec2::new(-5.0, 0.0)));
        assert_eq!(a[1], Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "colorShift": -40.0})));
        let a = t.pointer(&c, &ev(PointerKind::Drag, at + Vec2::new(-50.0, 0.0)));
        assert_eq!(a, vec![Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "colorShift": -100.0}))]);
    }

    /// The drag follows the spine's direction at the press: along a vertical spine, down adds.
    #[test]
    fn the_drag_follows_the_spines_direction() {
        let (d, id, s) = blend_doc(&[(200.0, 100.0), (200.0, 300.0)], eased());
        let p = paint();
        let c = cx(&d, &s, &p);
        let w = BlendWidget::of(&d, &s, 1.0, UP);
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w[1].at));
        let a = t.pointer(&c, &ev(PointerKind::Drag, w[1].at + Vec2::new(40.0, 10.0)));
        assert_eq!(a[1], Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "ramp": 50.0})));
    }

    /// Switching away from the Selection or Blend tool during a drag cancels it, and after a press
    /// that has not moved there is nothing to put back.
    #[test]
    fn a_tool_switch_cancels_the_drag() {
        let (d, _, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        let p = paint();
        let c = cx(&d, &s, &p);
        let at = Point::new(200.0, 216.0);
        let tools: [Box<dyn Tool>; 2] = [Box::new(SelectionTool::default()), Box::new(crate::meshblend::BlendTool::default())];
        for mut t in tools {
            t.pointer(&c, &ev(PointerKind::Down, at));
            assert_eq!(t.pointer(&c, &ev(PointerKind::Drag, at + Vec2::new(20.0, 0.0))).len(), 2);
            assert_eq!(t.deactivate(&c), vec![Action::Cancel]);
            assert!(!t.busy() && !t.transforming());
            t.pointer(&c, &ev(PointerKind::Down, at));
            assert!(t.busy());
            assert!(t.deactivate(&c).is_empty());
            assert!(!t.busy());
        }
    }

    /// The arrow keys during a drag change the value by one, ten with Shift, and Escape cancels.
    #[test]
    fn arrow_keys_step_and_escape_cancels() {
        let (d, id, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        let p = paint();
        let c = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        let at = Point::new(200.0, 216.0);
        t.pointer(&c, &ev(PointerKind::Down, at));
        let a = t.key(&c, ToolKey::Up, Mods::default());
        assert_eq!(a, vec![Action::Begin(LABEL.into()), Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "ramp": 41.0}))]);
        let a = t.key(&c, ToolKey::Left, Mods { shift: true, ..Default::default() });
        assert_eq!(a, vec![Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "ramp": 31.0}))]);
        for _ in 0..10 {
            t.key(&c, ToolKey::Down, Mods { shift: true, ..Default::default() });
        }
        let a = t.key(&c, ToolKey::Right, Mods::default());
        assert_eq!(a, vec![Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "ramp": 1.0}))], "steps past 0 are not kept");
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert!(!t.busy() && !t.transforming());
        assert!(t.pointer(&c, &ev(PointerKind::Up, at)).is_empty(), "the release commits nothing");
    }

    /// A double-click on a widget with the Selection tool does nothing to the artwork under it.
    #[test]
    fn a_double_click_on_a_widget_leaves_the_artwork_under_it_alone() {
        let (mut d, _, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        cover(&mut d, 190.0, 206.0, 210.0, 226.0);
        let p = paint();
        let c = cx(&d, &s, &p);
        let at = Point::new(200.0, 216.0);
        assert!(over_widget(&c, at));
        let mut t = SelectionTool::default();
        assert!(t.pointer(&c, &ev(PointerKind::DoubleClick, at)).is_empty());
        // Beside the widget, the double-click reaches the artwork.
        assert!(!t.pointer(&c, &ev(PointerKind::DoubleClick, Point::new(192.0, 224.0))).is_empty());
    }

    /// After a drag past either end, the first arrow key steps back from the value shown.
    #[test]
    fn an_arrow_key_after_a_drag_past_an_end_steps_from_the_value_shown() {
        let (d, id, s) = blend_doc(&[(100.0, 200.0), (300.0, 200.0)], eased());
        let p = paint();
        let c = cx(&d, &s, &p);
        let at = Point::new(200.0, 216.0);
        let ramp = |v: f64| vec![Action::Preview("object.blend.setEasing".into(), json!({"id": id.0, "ramp": v}))];
        for (moved, back, shown) in [(200.0, ToolKey::Left, 99.0), (-200.0, ToolKey::Right, 1.0)] {
            let mut t = SelectionTool::default();
            t.pointer(&c, &ev(PointerKind::Down, at));
            t.pointer(&c, &ev(PointerKind::Drag, at + Vec2::new(moved, 0.0)));
            assert_eq!(t.key(&c, back, Mods::default()), ramp(shown), "{moved}");
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, at + Vec2::new(moved, 0.0))), vec![Action::Commit]);
        }
    }
}
