//! The bounding box, Live Corners and the shape widgets drawn over the selection: the type, side and pie
//! widgets, the blend widgets and the Objects on Path widgets. They are display only: the Selection and shape
//! tools take the presses and drag them, so every control here declares no part and takes no key.

use craft::behavior::geometry::quad::BoxState;
use craft::behavior::kurbo::{Point, Rect};
use craft::behavior::{Input, Metrics};
use craft::draw::ui::corner::{CornerPoint, CornerTarget};
use craft::draw::ui::transform::TransformReach;
use craft::draw::ui::{CornerKind, CornerWidgets, TransformBox};
use craft::ui::canvas::canvas_space;
use craft::ui::canvas_controls::path_runs;
use craft::ui::contract::{Access, Bind, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _};
use craft::ui::primitives::{DataWidth, PaintShape, StrokeColor, StrokeWidth};
use craft::ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{Color32, Pos2, Response, vec2};
use vectorcraft_doc::{Document, LiveCorners, LiveShape, NodeId, NodeKind, OrientedBox};
use vectorcraft_geom::shapes::CornerKind as DocCornerKind;
use vectorcraft_tools::{blendwidget, objects_on_path, pie, sidewidget, typewidget};

use super::BodyKind;
use crate::VectorcraftApp;
use crate::doc_windows::Win;

/// How near the pointer comes to a Live Corners widget, in screen points, for the corner it controls to be
/// highlighted: the distance at which a press picks the widget.
const CORNER_HOVER_REACH: f64 = 5.0;
/// The width, in screen points, of the cut corner drawn while the pointer is over its Live Corners widget.
const CORNER_HOVER_WIDTH: f32 = 2.0;

/// The bounding box on show: the box and the layer colour it is drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ShownBox {
    /// The box in its own turned frame.
    pub(super) obox: OrientedBox,
    /// The layer colour of the box's first object, or of the pending preview's.
    pub(super) color: DataColor,
}

/// A layer colour as the framework's data colour: the one conversion of the widgets here.
fn data_color(rgb: [u8; 3]) -> DataColor {
    DataColor::new(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
}

/// A document point in the framework's geometry.
fn doc_point(p: vectorcraft_geom::Point) -> Point {
    Point::new(p.x, p.y)
}

/// Is the active tool a shape tool that is not drawing a shape, and `tool`, the tool whose widgets the canvas shows,
/// is that tool (Cmd lends no other)? While it draws it shows none of its widgets; adjusting a widget, it shows them.
fn shape_tool_idle(app: &VectorcraftApp, tool: &str) -> bool {
    tool == app.session.tool_id()
        && vectorcraft_tools::shape::shows_shape_widgets(tool)
        && !(app.session.tool_busy() && !app.session.tool_transforming())
}

/// The bounding box to draw, if any: the Selection tool's (rotated with the objects after a rotation), also
/// shown by the shape tools that keep a shape just drawn ready to adjust, and by Cmd held over another tool
/// where it lends the Selection tool to a press. It hides while a drag moves, rotates or resizes the selection, so
/// only the art is seen going with the pointer; a handle resize keeps it at the changing bounds. With General ›
/// Show Bounding Box While Moving and Drawing on, a move draws it where the drag has moved the selection and a
/// shape tool draws it around the shape being drawn. With Real-time Drawing off the document stays at the
/// drag's snapshot, so a resize or a shape being drawn measures the box from the pending preview; a shape being
/// drawn shows no box until a preview has made it. `tool` is the tool whose marks the canvas shows
/// ([`super::shown_tool`]): while Cmd is held, the selection tool a press borrows, so the box shows only when that
/// is the Selection tool, whose handles take the press.
// PLAN: the sheet's `(app, st)` cannot work: `selection_box` and `deferred_transform_box` need `&mut app`, so the
// active document is read inside.
pub(super) fn bounding_box(app: &mut VectorcraftApp, tool: &str) -> Option<ShownBox> {
    let resizing = app.session.tool_resizing();
    let move_box = app.session.move_drag_box();
    let drawing_boxed = app.session.draws_new_shape_box();
    let follows = resizing || move_box.is_some() || drawing_boxed;
    let own = tool == app.session.tool_id();
    let shape_tool = own && vectorcraft_tools::shape::shows_shape_widgets(tool);
    let show_box = (tool == "selection" || (shape_tool && (!app.session.tool_busy() || follows)))
        && (follows || !app.session.tool_transforming())
        && app.ui.view.bounding_box;
    if !show_box {
        return None;
    }
    let deferred = if resizing || drawing_boxed { app.session.deferred_transform_box() } else { None };
    let st = app.session.active()?;
    let selected_whole = !st.selection.is_empty() && st.selection.anchors.is_empty();
    let first = st.selection.objects.first().map(|id| st.doc.layer_color(*id));
    let (obox, color) = if let Some((b, color)) = deferred {
        (b, Some(color))
    } else if !selected_whole || (drawing_boxed && app.session.interaction_unapplied()) {
        return None;
    } else if let Some(b) = move_box {
        (b, first)
    } else {
        (app.selection_box()?, first)
    };
    Some(ShownBox { obox, color: data_color(color?) })
}

/// The corners of path `id` listed at `indices` (anchor indices of its uncut outline), as the framework's
/// target: each with its neighbours along the sharp outline, in the document.
fn corner_target(doc: &Document, id: NodeId, indices: &[usize]) -> Option<CornerTarget> {
    let node = doc.node(id)?;
    let lc = LiveCorners::of(node)?;
    let anchors: Vec<_> = lc.base.anchors().map(|(_, _, a)| doc_point(lc.xf * a.p)).collect();
    let corners: Vec<_> = indices
        .iter()
        .filter_map(|&i| {
            let c = lc.corner(i)?;
            Some(CornerPoint::new(
                doc_point(lc.xf * c.at),
                *anchors.get(c.before)?,
                *anchors.get(c.after)?,
                lc.drawn_radius(i),
                corner_kind(lc.kind(i)),
            ))
        })
        .collect();
    let individual = matches!(&node.kind, NodeKind::Path { live, .. } if matches!(live.as_deref(), Some(LiveShape::Rectangle { .. })));
    (!corners.is_empty()).then(|| CornerTarget::new(id.0, corners, individual))
}

/// The Live Corners widgets `tool` shows at `zoom`: Selection and the shape tools on a live rectangle or polygon
/// while Show Shape Widgets is on, Direct Selection on any path, within the Hide Corner Widget angle. `tool` is the
/// tool whose widgets the canvas shows ([`super::shown_tool`]).
fn shown_corners(app: &VectorcraftApp, tool: &str, zoom: f64) -> Option<vectorcraft_tools::corners::CornerWidgets> {
    let direct = tool == "directSelection";
    if !(matches!(tool, "selection" | "directSelection") || shape_tool_idle(app, tool)) {
        return None;
    }
    let st = app.session.active()?;
    let on = app.ui.view.corner_widgets && (direct || app.ui.view.shape_widgets);
    vectorcraft_tools::corners::CornerWidgets::showing(&st.doc, &st.selection, zoom, direct, on, app.session.prefs.hide_corner_widget_above)
}

/// The Live Corners widgets on show, one group per layer colour so each draws in its object's colour: Selection
/// and the shape tools on a live rectangle or polygon while Show Shape Widgets is on, Direct Selection on any
/// path. Only the corners the tool hit-tests are listed, those within the Hide Corner Widget angle. `tool` is the tool
/// whose widgets the canvas shows ([`super::shown_tool`]).
// PLAN: the decision's `(.., f64)` is dropped: the corners listed are already those within the angle, so no
// angle is left to hand on.
pub(super) fn corners(app: &VectorcraftApp, tool: &str, zoom: f64) -> Option<Vec<(DataColor, Vec<CornerTarget>)>> {
    let shown = shown_corners(app, tool, zoom)?;
    let st = app.session.active()?;
    // `visible_on` and `corners` list the same corners in the same order: path after path.
    let mut per_path: Vec<(NodeId, Vec<usize>)> = vec![];
    for ((id, _), index) in shown.visible_on().zip(shown.corners()) {
        match per_path.last_mut() {
            Some((last, indices)) if *last == id => indices.push(index),
            _ => per_path.push((id, vec![index])),
        }
    }
    let mut groups: Vec<(DataColor, Vec<CornerTarget>)> = vec![];
    for (id, indices) in per_path {
        let Some(target) = corner_target(&st.doc, id, &indices) else { continue };
        let color = data_color(st.doc.layer_color(id));
        match groups.iter_mut().find(|(c, _)| *c == color) {
            Some((_, targets)) => targets.push(target),
            None => groups.push((color, vec![target])),
        }
    }
    Some(groups)
}

/// The cut of the corner whose Live Corners widget is on show within 5 screen points of `at` (the distance at which
/// a press picks the widget), with its layer colour: what the canvas highlights under the pointer. `None` while the
/// tool is busy or a mouse button is down (`button_down`; a pan drag leaves the tool idle), away from the widgets,
/// and for a square corner, which has no cut. `tool` is the tool whose widgets the canvas shows.
pub(super) fn corner_hover(
    app: &VectorcraftApp,
    tool: &str,
    zoom: f64,
    at: Option<vectorcraft_geom::Point>,
    button_down: bool,
) -> Option<(DataColor, vectorcraft_geom::BezPath)> {
    if button_down || app.session.tool_busy() {
        return None;
    }
    let shown = shown_corners(app, tool, zoom)?;
    let st = app.session.active()?;
    let (id, cut) = shown.hovered_cut(&st.doc, at?, CORNER_HOVER_REACH / zoom.max(1e-9))?;
    Some((data_color(st.doc.layer_color(id)), cut))
}

fn corner_kind(kind: DocCornerKind) -> CornerKind {
    match kind {
        DocCornerKind::Round => CornerKind::Round,
        DocCornerKind::InvertedRound => CornerKind::InvertedRound,
        DocCornerKind::Chamfer => CornerKind::Chamfer,
    }
}

/// The outline of a mark.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Form {
    Disc,
    Diamond,
    Square,
}

/// How a mark is filled.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Filling {
    /// The handle fill, outlined in the layer colour.
    Hollow,
    /// The layer colour.
    Layer,
    /// A ring in the layer colour with nothing inside.
    Ring,
    /// The accent, outlined in the handle fill: the chosen object's Select/Move widget.
    Chosen,
}

/// One widget: where it sits in the document, how it is drawn and what it is joined to.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mark {
    form: Form,
    filling: Filling,
    at: Point,
    /// A point a line joins the widget to.
    join: Option<Point>,
    color: DataColor,
    /// The radius (half the side of a square) in screen points.
    radius: f32,
}

impl Mark {
    fn new(form: Form, filling: Filling, at: vectorcraft_geom::Point, radius: f32, color: DataColor) -> Self {
        Mark { form, filling, at: doc_point(at), join: None, color, radius }
    }

    fn joined(mut self, to: vectorcraft_geom::Point) -> Self {
        self.join = Some(doc_point(to));
        self
    }

    fn shape(&self, at: Pos2) -> PaintShape {
        let r = self.radius;
        match self.form {
            Form::Disc => PaintShape::Circle(at, r),
            Form::Diamond => PaintShape::Polygon(vec![at + vec2(0.0, -r), at + vec2(r, 0.0), at + vec2(0.0, r), at + vec2(-r, 0.0)]),
            Form::Square => PaintShape::Rect(egui::Rect::from_center_size(at, vec2(2.0 * r, 2.0 * r)), egui::CornerRadius::ZERO),
        }
    }
}

/// The type, side, pie, blend and Objects on Path widgets, drawn as one paint-only canvas control.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
pub struct ShapeWidgets {
    #[common]
    common: Common,
    marks: Vec<Mark>,
}

impl ShapeWidgets {
    /// The widgets of the active document at `zoom` (screen points per document unit) under a view turned by
    /// `rotation`. `edges` is View › Show Edges: the type, side and pie widgets go with it, the blend and Objects
    /// on Path widgets are controls and stay. `shown` is the bounding box on show, which the type and side
    /// widgets sit beside; a shape being drawn has neither. `tool` is the tool whose widgets the canvas shows
    /// ([`super::shown_tool`]): while Cmd is held, the selection tool a press borrows.
    fn of(app: &VectorcraftApp, tool: &str, edges: bool, shown: Option<&ShownBox>, zoom: f64, rotation: f64) -> Self {
        let mut marks = vec![];
        let Some(st) = app.session.active() else { return Self::new(marks) };
        let layer = |id| data_color(st.doc.layer_color(id));
        let shape_widgets = app.ui.view.shape_widgets;
        if edges {
            if let Some(b) = shown.filter(|_| !app.session.draws_new_shape_box()) {
                if let Some(w) = typewidget::TypeWidget::of(&st.doc, &st.selection, &b.obox, zoom) {
                    let filling = if w.area { Filling::Layer } else { Filling::Hollow };
                    marks.push(Mark::new(Form::Disc, filling, w.at, typewidget::RADIUS_PX, b.color));
                }
                if shape_widgets && let Some(w) = sidewidget::SideWidget::of(&st.doc, &st.selection, &b.obox, zoom) {
                    marks.push(Mark::new(Form::Diamond, Filling::Hollow, w.at, sidewidget::RADIUS_PX, b.color));
                }
            }
            // Pie widgets stay while one is dragged, each joined to the outline's point at its angle.
            if app.ui.view.bounding_box && shape_widgets && (tool == "selection" || shape_tool_idle(app, tool)) {
                for w in pie::PieWidget::of(&st.doc, &st.selection, zoom) {
                    marks.push(Mark::new(Form::Disc, Filling::Hollow, w.at, pie::RADIUS_PX, layer(w.id)).joined(w.on));
                }
            }
        }
        if matches!(tool, "selection" | "blend") {
            // The screen's up in the document, with the view turned by `rotation`.
            let up = vectorcraft_geom::Vec2::new(-rotation.sin(), -rotation.cos());
            for w in blendwidget::BlendWidget::of(&st.doc, &st.selection, zoom, up) {
                let filling = if w.control == blendwidget::BlendControl::Easing { Filling::Layer } else { Filling::Hollow };
                marks.push(Mark::new(Form::Disc, filling, w.at, blendwidget::RADIUS_PX, layer(w.id)).joined(w.on));
            }
        }
        if matches!(tool, "selection" | "objectsOnPath") {
            let widgets = objects_on_path::Widgets::of(&st.doc, &st.selection, zoom);
            // The Select/Move widgets first: they draw under the others, which a press takes first.
            for w in &widgets.members {
                let filling = if w.chosen { Filling::Chosen } else { Filling::Ring };
                marks.push(Mark::new(Form::Disc, filling, w.at, objects_on_path::MEMBER_RADIUS_PX, layer(w.group)));
            }
            if let Some(w) = widgets.rotate_all {
                marks.push(Mark::new(Form::Disc, Filling::Hollow, w.at, objects_on_path::ROTATE_ALL_RADIUS_PX, layer(w.group)).joined(w.pivot));
            }
            if let Some(w) = widgets.space {
                marks.push(Mark::new(Form::Square, Filling::Hollow, w.at, objects_on_path::SPACE_HALF_PX, layer(w.group)).joined(w.last));
            }
            if let Some(w) = widgets.move_all {
                marks.push(Mark::new(Form::Diamond, Filling::Hollow, w.at, objects_on_path::MOVE_ALL_HALF_PX, layer(w.group)).joined(w.first));
            }
        }
        Self::new(marks)
    }

    fn new(marks: Vec<Mark>) -> Self {
        ShapeWidgets { common: Common::new(), marks }.label("Shape widgets")
    }
}

/// The style of the shape widgets.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ShapeWidgetsStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The inside of a hollow widget and the outline of the chosen one: `overlay.handle_fill`.
    fill: StyleColor,
    /// The inside of the chosen Select/Move widget: `overlay.line`.
    chosen: StyleColor,
}

impl Component for ShapeWidgets {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = ShapeWidgetsStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ShapeWidgetsStyleKey) -> ShapeWidgetsStyle {
        let overlay = &cx.tokens.color.overlay;
        ShapeWidgetsStyle { metrics: Metrics::new(), fill: cx.color(overlay.handle_fill), chosen: cx.color(overlay.line) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let draw = ui.draw();
        for m in &self.marks {
            let at = space.view.to_screen(m.at);
            if let Some(from) = m.join {
                draw.stroke(&PaintShape::Path(vec![space.view.to_screen(from), at]), StrokeWidth::Thin, StrokeColor::Data(m.color));
            }
            let shape = m.shape(at);
            match m.filling {
                Filling::Hollow => draw.fill(&shape, cx.style.fill),
                Filling::Layer => draw.data_fill(&shape, m.color),
                Filling::Chosen => draw.fill(&shape, cx.style.chosen),
                Filling::Ring => {}
            }
            let (width, ink) = match m.filling {
                Filling::Chosen => (StrokeWidth::Thin, StrokeColor::Theme(cx.style.fill)),
                Filling::Ring => (StrokeWidth::Medium, StrokeColor::Data(m.color)),
                Filling::Hollow | Filling::Layer => (StrokeWidth::Thin, StrokeColor::Data(m.color)),
            };
            draw.stroke(&shape, width, ink);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.description("Shape widgets");
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The cut of the corner whose Live Corners widget the pointer is over, drawn under the widgets in its layer colour
/// as one paint-only canvas control.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
pub struct CornerHover {
    #[common]
    common: Common,
    cut: vectorcraft_geom::BezPath,
    color: DataColor,
}

impl CornerHover {
    fn new(cut: vectorcraft_geom::BezPath, color: DataColor) -> Self {
        CornerHover { common: Common::new(), cut, color }.label("Corner highlight")
    }
}

/// The style of the corner highlight.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CornerHoverStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl Component for CornerHover {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = CornerHoverStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: CornerHoverStyleKey) -> CornerHoverStyle {
        CornerHoverStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let draw = ui.draw();
        for (points, _) in path_runs(&space.view, &self.cut) {
            draw.stroke(&PaintShape::Path(points), DataWidth::points(CORNER_HOVER_WIDTH), StrokeColor::Data(self.color));
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.description("Corner highlight");
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Shows the bounding box, Live Corners and the shape widgets over the selection of the live document. The box,
/// Live Corners, type, side and pie widgets need View › Show Edges; the blend and Objects on Path widgets do not.
/// Nothing here is shown while a revolve is edited.
// PLAN: `win` is unused: assumed it names the canvas window of the pass, and only a `Live` body shows these.
pub(super) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, space: &craft::ui::canvas::CanvasSpace, _win: Win, kind: BodyKind) {
    if kind != BodyKind::Live || super::revolve_editing(app) {
        return;
    }
    // PLAN: assumed the body's rect is the canvas rect the controls are shown at.
    let rect = ui.max_rect();
    let edges = app.ui.view.edges;
    // While Cmd is held, the widgets of the selection tool a press borrows.
    let tool = super::shown_tool(app, ui.ctx());
    let shown = if edges { bounding_box(app, tool) } else { None };
    if edges {
        // The pointer of this pass: `hover_doc` is only set after the canvas body has been shown.
        let (pointer, button_down) = ui.input(|i| (i.pointer.hover_pos(), i.pointer.any_down()));
        let at = pointer.filter(|_| ui.rect_contains_pointer(rect)).map(|p| space.view.to_doc(p));
        if let Some((color, cut)) = corner_hover(app, tool, space.view.zoom, at, button_down) {
            CornerHover::new(cut, color).show_at(ui, rect);
        }
        for (at, (color, targets)) in corners(app, tool, space.view.zoom).into_iter().flatten().enumerate() {
            CornerWidgets::new("Live Corners", Bind::given(targets)).id_salt(at).reach(TransformReach::Shown).color(color).show_at(ui, rect);
        }
        if let Some(b) = &shown {
            show_box(ui, rect, b);
        }
    }
    let widgets = ShapeWidgets::of(app, tool, edges, shown.as_ref(), space.view.zoom, space.view.rotation);
    if !widgets.marks.is_empty() {
        widgets.show_at(ui, rect);
    }
}

/// The box and its eight handles, drawn in the layer colour and answering nothing.
fn show_box(ui: &mut egui::Ui, rect: egui::Rect, b: &ShownBox) {
    let state = BoxState::new(b.obox.corners().map(doc_point), doc_point(b.obox.center()));
    let r = b.obox.rect;
    TransformBox::new("Bounding box", Bind::given(state), Rect::new(r.x0, r.y0, r.x1, r.y1), 0.0)
        .reach(TransformReach::Shown)
        .reference_point(false)
        .color(b.color)
        .show_at(ui, rect);
}

#[cfg(test)]
mod tests;
