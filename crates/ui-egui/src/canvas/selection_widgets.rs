//! The Selection tool's on-canvas widgets as one canvas control: the symbol 9-slice guides, the Shaper mode widget,
//! the live mirror axis, the Image Trace split divider and the grid and radial repeat handles.
//!
//! Each widget is a set of parts in the canvas's press arena, so a press on one stays with the control and one
//! elsewhere reaches the tool. A part exists only where the Selection tool would let the widget take the press: a
//! bounding-box handle or a shown centre point keeps its own drag (the parts cut those discs out of their shapes),
//! and a press with Cmd, Ctrl or Alt held belongs to the tool's own gestures. The control paints, declares and
//! reports presses; [`show`] runs the modules' drag values with the presses and sends their actions to the session,
//! where one `Begin`..`Commit` is one undo step.
//!
//! PLAN: `mod selection_widgets;` in `canvas.rs` is canvas-w10's. The `craft::ui::{contract, canvas, primitives,
//! style}` and `craft::behavior` paths are assumed to be the framework crates' public paths for the items used.

use craft::behavior::{CanvasView, Cursor, HitLevel, HitPriority, HitShape, Metrics, PartKey, PointerButton, PressPhase, PressState, Sense};
use craft::ui::canvas::CanvasSpace;
use craft::ui::contract::{Common, Component, Cx, MeasureCx, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, Show as _};
use craft::ui::primitives::{PaintShape, StrokeWidth};
use craft::ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2};
use vectorcraft_geom::{Point, Vec2 as DocVec};
use vectorcraft_tools::{Action, Mods, PointerEvent, PointerKind, ToolContext};
use vectorcraft_tools::{mirror, repeat_grid, repeat_radial, select, shaperwidget, trace_split};

use super::BodyKind;
use crate::VectorcraftApp;
use crate::cursors::tool_cursor;

#[cfg(test)]
mod tests;

/// Where a press on a bounding-box handle still counts: the circle around the handle that holds its square reach.
const BOX_HANDLE_REACH: f32 = 5.0 * std::f32::consts::SQRT_2;

/// Which widgets the app's state shows, before the document says which exist.
#[derive(Clone, Copy, Debug)]
pub(super) struct Show {
    /// The 9-slice guides: with any tool while a symbol is edited.
    pub nine: bool,
    /// The Selection tool with selection edges shown: the repeat widgets.
    pub selecting: bool,
    /// The Shaper mode widget: the Shaper and Selection tools.
    pub shaper: bool,
    /// The trace divider: `selecting`, and not in outline view.
    pub trace: bool,
}

/// One widget of the control; its parts share a [`PartKey`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Widget {
    Nine,
    Shaper,
    Mirror,
    Grid,
    Radial,
    Trace,
}

impl Widget {
    const fn key(self) -> PartKey {
        match self {
            Widget::Nine => PartKey::new("nine_slice"),
            Widget::Shaper => PartKey::new("shaper"),
            Widget::Mirror => PartKey::new("mirror"),
            Widget::Grid => PartKey::new("grid_repeat"),
            Widget::Radial => PartKey::new("radial_repeat"),
            Widget::Trace => PartKey::new("trace_split"),
        }
    }

    /// Whether a press with Cmd, Ctrl or Alt held still takes the widget; the modules refuse those for the others.
    const fn takes_modified(self) -> bool {
        matches!(self, Widget::Nine | Widget::Shaper)
    }
}

/// A part to declare: where it is hit, and the cursor over it.
#[derive(Clone, Debug)]
struct Item {
    widget: Widget,
    index: u64,
    hit: HitShape,
    cursor: Cursor,
    priority: u8,
}

/// A grid control as drawn: its centre, and the bar a spacing slider is.
#[derive(Clone, Debug)]
struct GridMark {
    at: Pos2,
    bar: Option<[Pos2; 2]>,
}

/// A radial repeat as drawn: the circle the instances sit on, and its handles, the splitter with its line across.
#[derive(Clone, Debug)]
struct RadialMark {
    circle: Option<(Pos2, f32)>,
    handles: Vec<(Pos2, Option<[Pos2; 2]>)>,
}

/// The Shaper widget as drawn.
#[derive(Clone, Debug)]
struct ShaperMark {
    at: Pos2,
    construction: bool,
    color: DataColor,
}

/// Everything the control draws and declares in one pass, in screen points.
#[derive(Clone, Debug, Default)]
pub(super) struct Layout {
    nine: Vec<[Pos2; 2]>,
    shaper: Option<ShaperMark>,
    mirror: Option<([Pos2; 2], Pos2)>,
    trace: Option<[Pos2; 2]>,
    grid: Vec<GridMark>,
    radial: Option<RadialMark>,
    items: Vec<Item>,
}

fn doc_rgb(rgb: [u8; 3]) -> DataColor {
    DataColor::new(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
}

impl Layout {
    fn item(&mut self, widget: Widget, hit: HitShape, cursor: Cursor, priority: u8) {
        let index = self.items.len() as u64;
        self.items.push(Item { widget, index, hit, cursor, priority });
    }

    /// Whether `widget` has a part to press.
    fn shows(&self, widget: Widget) -> bool {
        self.items.iter().any(|item| item.widget == widget)
    }

    /// The widgets `show` allows, placed through `view`, with the parts the tool would let take a press.
    pub(super) fn of(cx: &ToolContext, view: &CanvasView, show: Show) -> Self {
        let mut l = Layout::default();
        let at = |p: Point| view.to_screen(p);
        let boxes: Vec<(Pos2, f32)> = select::bbox_handle_points(cx).into_iter().map(|p| (at(p), BOX_HANDLE_REACH)).collect();
        let centre_reach = (cx.point_tol() * cx.zoom) as f32;
        let centres = select::shown_centers(cx).into_iter().map(|(_, p)| (at(p), centre_reach));
        let move_cursor = tool_cursor(vectorcraft_tools::Cursor::Move);

        if show.nine
            && let Some(e) = cx.doc.edited_nine_slice()
        {
            for (_, a, b) in e.lines() {
                let ends = [at(a), at(b)];
                if ends.iter().all(|p| p.is_finite()) {
                    l.nine.push(ends);
                    l.item(Widget::Nine, HitShape::polyline(ends.to_vec(), false), move_cursor.clone(), 0);
                }
            }
        }
        if show.shaper
            && let Some(w) = shaperwidget::ShaperWidget::of(cx.doc, cx.selection, cx.isolation, cx.zoom)
            && at(w.at).is_finite()
        {
            let c = at(w.at);
            l.shaper = Some(ShaperMark { at: c, construction: w.construction, color: doc_rgb(cx.doc.layer_color(w.group)) });
            let arrow = tool_cursor(vectorcraft_tools::Cursor::Arrow);
            l.item(Widget::Shaper, HitShape::circle(c, shaperwidget::RADIUS_PX + 1.0), arrow, 0);
        }
        if show.selecting && cx.show_bbox {
            l.repeat_widgets(cx, view, &boxes);
            if show.trace && !cx.outline {
                l.trace_divider(cx, view, boxes.iter().copied().chain(centres).collect());
            }
        }
        l
    }

    fn repeat_widgets(&mut self, cx: &ToolContext, view: &CanvasView, boxes: &[(Pos2, f32)]) {
        let at = |p: Point| view.to_screen(p);
        if let Some(axis) = mirror::MirrorAxis::of(cx.doc, cx.selection, cx.zoom) {
            let (ends, centre) = (axis.ends.map(at), at(axis.center));
            if centre.is_finite() && ends.iter().all(|p| p.is_finite()) {
                self.mirror = Some((ends, centre));
                let line = HitShape::polyline_within(ends.to_vec(), false, mirror::REACH);
                self.item(Widget::Mirror, HitShape::excluding(line, boxes.to_vec()), tool_cursor(vectorcraft_tools::Cursor::Move), 0);
                for (end, doc_end) in ends.iter().zip(axis.ends) {
                    if let Some(c) = mirror::cursor(cx, doc_end) {
                        self.item(Widget::Mirror, HitShape::circle(*end, mirror::END_REACH), tool_cursor(c), 1);
                    }
                }
                self.item(Widget::Mirror, HitShape::circle(centre, mirror::REACH), tool_cursor(vectorcraft_tools::Cursor::Move), 1);
            }
        }
        if let Some(g) = repeat_grid::GridHandles::of(cx.doc, cx.selection, select::selection_box(cx), cx.zoom) {
            let half = f64::from(repeat_grid::SLIDER_HALF) / cx.zoom;
            for (part, p) in g.parts() {
                let c = at(p);
                if !c.is_finite() {
                    continue;
                }
                let bar = part.is_slider().then(|| {
                    let along = if part == repeat_grid::GridPart::HSpacing { DocVec::new(0.0, half) } else { DocVec::new(half, 0.0) };
                    [at(p - along), at(p + along)]
                });
                self.grid.push(GridMark { at: c, bar });
                if let Some(cursor) = repeat_grid::cursor(cx, p) {
                    let disc = HitShape::circle(c, repeat_grid::REACH as f32);
                    self.item(Widget::Grid, HitShape::excluding(disc, boxes.to_vec()), tool_cursor(cursor), 1);
                }
            }
        }
        if let Some(h) = repeat_radial::RadialHandles::of(cx.doc, cx.selection, select::selection_box(cx), cx.zoom) {
            let centre = at(h.center);
            let r = (h.radius * cx.zoom) as f32;
            let circle = (centre.is_finite() && r.is_finite() && r > 0.5 && r < 20_000.0).then_some((centre, r));
            let mut mark = RadialMark { circle, handles: vec![] };
            for (part, p) in h.parts() {
                let c = at(p);
                if !c.is_finite() {
                    continue;
                }
                let across = (c - centre).normalized() * (repeat_radial::HANDLE_RADIUS + 4.0);
                let bar = (part == repeat_radial::RadialPart::Splitter && across.is_finite()).then_some([c - across, c + across]);
                mark.handles.push((c, bar));
                if let Some(cursor) = repeat_radial::cursor(cx, p) {
                    let disc = HitShape::circle(c, repeat_radial::REACH as f32);
                    self.item(Widget::Radial, HitShape::excluding(disc, boxes.to_vec()), tool_cursor(cursor), 1);
                }
            }
            self.radial = Some(mark);
        }
    }

    fn trace_divider(&mut self, cx: &ToolContext, view: &CanvasView, holes: Vec<(Pos2, f32)>) {
        let Some(d) = trace_split::TraceDivider::of(cx.doc, cx.selection, cx.zoom) else { return };
        let ends = d.ends.map(|p| view.to_screen(p));
        if ends.iter().all(|p| p.is_finite()) {
            self.trace = Some(ends);
            let line = HitShape::polyline_within(ends.to_vec(), false, trace_split::HIT_PX as f32);
            self.item(Widget::Trace, HitShape::excluding(line, holes), tool_cursor(trace_split::resize_cursor(&d, cx)), 0);
        }
    }
}

/// The style of the widgets: a halo under the accent, and the size of a handle.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(super) struct WidgetStyle {
    #[style(metrics)]
    metrics: Metrics,
    halo: StyleColor,
    accent: StyleColor,
    handle: f32,
}

/// The selection widgets of a canvas.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
pub(super) struct SelectionWidgets {
    #[common]
    common: Common,
    layout: Layout,
    presses: Vec<(Widget, PressState)>,
}

impl SelectionWidgets {
    fn new(layout: Layout) -> Self {
        SelectionWidgets { common: Common::new(), layout, presses: vec![] }.label(tl!("Selection widgets"))
    }
}

/// What the control reports: the presses on its parts this pass.
pub(super) struct WidgetsOutput {
    response: Response,
    presses: Vec<(Widget, PressState)>,
}

impl Outcome for WidgetsOutput {
    type Value = ();

    fn response(&self) -> &Response {
        &self.response
    }
}

impl Component for SelectionWidgets {
    const ROLE: Role = Role::GROUP;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = WidgetStyle;
    type Output = WidgetsOutput;

    fn style(cx: &StyleCx<'_>, _: SelectionWidgetsStyleKey) -> WidgetStyle {
        let tokens = cx.tokens;
        WidgetStyle {
            metrics: Metrics::new(),
            halo: cx.color(tokens.color.overlay.halo),
            accent: cx.color(tokens.color.accent.solid),
            handle: tokens.size.handle.md,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// The parts of the widgets. The modules' drags refuse Cmd, Ctrl and Alt, so with one held (and no gesture of
    /// the control running) their parts are not declared and the press reaches the tool.
    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let mods = craft::ui::runtime::pointer::frame(cx.ui.ctx()).mods;
        let modified = (mods.cmd || mods.ctrl || mods.alt) && !cx.gesture_running();
        for item in self.layout.items.iter().filter(|item| item.widget.takes_modified() || !modified) {
            cx.part(item.widget.key().at(item.index), item.hit.clone())
                .sense(Sense::DRAG | Sense::CLICK)
                .priority(HitPriority::new(HitLevel::Edge, item.priority))
                .cursor(item.cursor.clone());
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        for item in &self.layout.items {
            if let Some(result) = cx.part_if_declared(item.widget.key().at(item.index))
                && result.press.phase.is_press()
            {
                self.presses.push((item.widget, result.press));
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (style, l) = (&cx.style, &self.layout);
        let r = style.handle / 2.0;
        let draw = ui.draw();
        draw.clipped(cx.rect, |draw| {
            for ends in &l.nine {
                draw.dashed(ends, draw.dashes(), StrokeWidth::Thin, style.accent);
            }
            if let Some(w) = &l.shaper {
                let circle = PaintShape::Circle(w.at, r);
                draw.fill(&circle, style.halo);
                draw.stroke(&circle, StrokeWidth::Thin, w.color);
                let d = if w.construction { -1.0 } else { 1.0 };
                let tip = w.at + Vec2::new(0.5 * r * d, 0.0);
                let back = [w.at + Vec2::new(-0.33 * r * d, -0.5 * r), w.at + Vec2::new(-0.33 * r * d, 0.5 * r)];
                draw.data_fill(&PaintShape::Polygon(vec![tip, back[0], back[1]]), w.color);
            }
            if let Some((ends, centre)) = &l.mirror {
                let line = PaintShape::Path(ends.to_vec());
                draw.stroke(&line, StrokeWidth::Thick, style.halo);
                draw.stroke(&line, StrokeWidth::Medium, style.accent);
                for end in ends {
                    let disc = PaintShape::Circle(*end, r);
                    draw.fill(&disc, style.halo);
                    draw.stroke(&disc, StrokeWidth::Medium, style.accent);
                }
                let square = PaintShape::Rect(Rect::from_center_size(*centre, Vec2::splat(style.handle)), CornerRadius::same(2));
                draw.fill(&square, style.accent);
                draw.stroke(&square, StrokeWidth::Medium, style.halo);
            }
            if let Some(ends) = &l.trace {
                let line = PaintShape::Path(ends.to_vec());
                draw.stroke(&line, StrokeWidth::Thick, style.halo);
                draw.stroke(&line, StrokeWidth::Thin, style.accent);
                let (mid, along) = (ends[0] + (ends[1] - ends[0]) * 0.5, ends[1] - ends[0]);
                let length = along.length();
                if length > 1.0 {
                    let half = along / length * trace_split::GRIP_HALF.min(length * 0.5);
                    let grip = PaintShape::Path(vec![mid - half, mid + half]);
                    draw.stroke(&grip, StrokeWidth::Heavy, style.halo);
                    draw.stroke(&grip, StrokeWidth::Thick, style.accent);
                }
            }
            for g in &l.grid {
                match g.bar {
                    Some(bar) => {
                        let bar = PaintShape::Path(bar.to_vec());
                        draw.stroke(&bar, StrokeWidth::Heavy, style.halo);
                        draw.stroke(&bar, StrokeWidth::Medium, style.accent);
                    }
                    None => handle(draw, g.at, r, style),
                }
            }
            if let Some(m) = &l.radial {
                if let Some((centre, radius)) = m.circle {
                    let ring = PaintShape::Circle(centre, radius);
                    draw.stroke(&ring, StrokeWidth::Thick, style.halo);
                    draw.stroke(&ring, StrokeWidth::Thin, style.accent);
                }
                for (at, bar) in &m.handles {
                    if let Some(bar) = bar {
                        let bar = PaintShape::Path(bar.to_vec());
                        draw.stroke(&bar, StrokeWidth::Thick, style.halo);
                        draw.stroke(&bar, StrokeWidth::Medium, style.accent);
                    }
                    handle(draw, *at, r, style);
                }
            }
        });
    }

    fn finish(self, cx: Cx<'_, Self>) -> WidgetsOutput {
        WidgetsOutput { response: cx.response.clone(), presses: self.presses }
    }
}

fn handle(draw: &craft::ui::primitives::Draw<'_>, at: Pos2, radius: f32, style: &WidgetStyle) {
    let disc = PaintShape::Circle(at, radius);
    draw.fill(&disc, style.halo);
    draw.stroke(&disc, StrokeWidth::Medium, style.accent);
}

/// The drag a press on a widget runs: the module's own value, which previews through the engine's grouping.
#[derive(Clone, Debug)]
pub(super) enum Drag {
    Mirror(mirror::MirrorDrag),
    Grid(repeat_grid::GridDrag),
    Radial(repeat_radial::RadialDrag),
    Trace(trace_split::SplitDrag),
    Nine(vectorcraft_tools::nineslice::GuideDrag),
}

impl Drag {
    /// The drag a press at `p` begins on `widget`, none where the module refuses it (modifiers, a bounding-box
    /// handle) or the widget is the Shaper's, which only clicks.
    fn hit(widget: Widget, cx: &ToolContext, p: Point, mods: Mods) -> Option<Self> {
        match widget {
            Widget::Mirror => mirror::MirrorDrag::hit(cx, p, mods).map(Drag::Mirror),
            Widget::Grid => repeat_grid::GridDrag::hit(cx, p, mods).map(Drag::Grid),
            Widget::Radial => repeat_radial::RadialDrag::hit(cx, p, mods).map(Drag::Radial),
            Widget::Trace => trace_split::SplitDrag::hit(cx, p, mods).map(Drag::Trace),
            Widget::Nine => vectorcraft_tools::nineslice::GuideDrag::hit(cx, &PointerEvent::new(PointerKind::Down, p.x, p.y)).map(Drag::Nine),
            Widget::Shaper => None,
        }
    }

    fn drag(&mut self, cx: &ToolContext, p: Point, mods: Mods) -> Vec<Action> {
        match self {
            Drag::Mirror(d) => d.drag(cx, p, mods),
            Drag::Grid(d) => d.drag(p),
            Drag::Radial(d) => d.drag(p),
            Drag::Trace(d) => d.drag(cx, p),
            Drag::Nine(d) => d.drag(cx, p),
        }
    }

    fn finish(self) -> Vec<Action> {
        match self {
            Drag::Mirror(d) => d.finish(),
            Drag::Grid(d) => d.finish(),
            Drag::Radial(d) => d.finish(),
            Drag::Trace(d) => d.finish(),
            Drag::Nine(d) => d.finish(),
        }
    }

    fn cancel(self) -> Vec<Action> {
        match self {
            Drag::Mirror(d) => d.cancel(),
            Drag::Grid(d) => d.cancel(),
            Drag::Radial(d) => d.cancel(),
            Drag::Trace(d) => d.cancel(),
            Drag::Nine(d) => d.cancel(),
        }
    }
}

/// A press that took a widget: the widget, the module's drag, and where the press last held the pointer.
#[derive(Clone, Debug)]
pub(super) struct Held {
    widget: Widget,
    drag: Drag,
    at: Point,
}

/// One step of a press on a widget, in document points.
#[derive(Clone, Copy, Debug)]
pub(super) enum Gesture {
    /// The button went down at the point.
    Press(Point),
    /// The pointer moved past the slop.
    Move(Point),
    /// The button went up at the point after a drag or a hold.
    Release(Point),
    /// The button went up where it went down: the Shaper widget's click.
    Click(Point),
    /// The press ended without a release (Escape, a lost pointer).
    Cancel,
}

/// The actions `g` gives for the press on `widget` that `held` holds, which this updates.
pub(super) fn plan(app: &mut VectorcraftApp, held: &mut Option<Held>, widget: Widget, g: Gesture, mods: Mods) -> Vec<Action> {
    let view = app.view_info();
    app.session.with_tool_context(view, |cx| match g {
        Gesture::Press(p) => {
            *held = Drag::hit(widget, cx, p, mods).map(|drag| Held { widget, drag, at: p });
            vec![]
        }
        Gesture::Move(p) => held
            .as_mut()
            .map(|h| {
                h.at = p;
                h.drag.drag(cx, p, mods)
            })
            .unwrap_or_default(),
        // Released away from where the press last held the pointer, the drag goes there first: the release keeps
        // what a drag to that point shows.
        Gesture::Release(p) => held
            .take()
            .map(|mut h| {
                let mut out = if h.at == p { vec![] } else { h.drag.drag(cx, p, mods) };
                out.extend(h.drag.finish());
                out
            })
            .unwrap_or_default(),
        Gesture::Cancel => held.take().map(|h| h.drag.cancel()).unwrap_or_default(),
        Gesture::Click(p) => shaperwidget::click(cx, p).into_iter().collect(),
    })
}

/// [`plan`], with the actions applied to the session.
pub(super) fn advance(app: &mut VectorcraftApp, held: &mut Option<Held>, widget: Widget, g: Gesture, mods: Mods) {
    let actions = plan(app, held, widget, g, mods);
    if !actions.is_empty() {
        let result = app.session.apply_actions(actions);
        super::apply_requests(app, result);
    }
}

/// Ends the drag `held` holds, as Escape does, when `layout` no longer shows its widget (another tool chosen, Edges or
/// the bounding box hidden): its value goes back.
pub(super) fn end_hidden(app: &mut VectorcraftApp, held: &mut Option<Held>, layout: &Layout) {
    if let Some(widget) = held.as_ref().map(|h| h.widget)
        && !layout.shows(widget)
    {
        advance(app, held, widget, Gesture::Cancel, Mods::default());
    }
}

/// Which widgets `tool` and the app's view show. `tool` is the tool whose widgets the canvas shows
/// ([`super::shown_tool`]): while Cmd is held, the selection tool a press borrows.
pub(super) fn shown(app: &VectorcraftApp, tool: &str) -> Show {
    let revolve_edit = super::revolve_editing(app);
    let selecting = tool == "selection" && app.ui.view.edges && !revolve_edit;
    Show { nine: true, selecting, shaper: matches!(tool, "shaper" | "selection") && !revolve_edit, trace: selecting && !app.outline_view() }
}

/// Shows the widgets over the document and runs the presses on them. Runs before the tool host, so a press on a part
/// never reaches the tool. While Cmd lends the Selection tool, its widgets show and take a press as that tool's,
/// without Cmd, and a press on one keeps them until the release.
pub(super) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, space: &CanvasSpace, win: super::Win, kind: BodyKind) {
    if kind != BodyKind::Live || app.ui.screen_mode == 3 {
        return;
    }
    let id = super::canvas_id(win).with("selection_widget_drag");
    let tool_id = id.with("tool");
    let mut held: Option<Held> = ui.data(|d| d.get_temp::<Option<Held>>(id)).flatten();
    let tool = match held {
        Some(_) => ui.data(|d| d.get_temp::<&'static str>(tool_id)),
        None => None,
    }
    .unwrap_or_else(|| super::shown_tool(app, ui.ctx()));
    let lent = tool != app.session.tool_id();
    let (visible, info) = (shown(app, tool), app.view_info());
    let layout = app.session.with_tool_context(info, |cx| Layout::of(cx, &space.view, visible));
    end_hidden(app, &mut held, &layout);
    let out = SelectionWidgets::new(layout).show_at(ui, space.view.rect);
    for (widget, press) in out.presses {
        if !press.phase.start().is_some_and(|s| s.button == PointerButton::Primary) {
            continue;
        }
        let g = match press.phase {
            PressPhase::Down { start, .. } => Gesture::Press(space.view.to_doc(start.origin)),
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => Gesture::Move(space.view.to_doc(pos)),
            PressPhase::Release { pos, click, .. } if widget == Widget::Shaper && click => Gesture::Click(space.view.to_doc(pos)),
            PressPhase::Release { pos, .. } => Gesture::Release(space.view.to_doc(pos)),
            PressPhase::Cancel { .. } => Gesture::Cancel,
            _ => continue,
        };
        let mut mods = super::tool_adapter::tool_mods(press.mods, false);
        // The lent tool takes the press as a plain one, as the session lends it.
        mods.cmd &= !lent;
        advance(app, &mut held, widget, g, mods);
    }
    ui.data_mut(|d| {
        d.insert_temp(id, held);
        d.insert_temp(tool_id, tool);
    });
}
