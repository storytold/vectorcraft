//! The selection's marks over the document: outlines, anchors and handles, the hover outline, and the dots (the
//! centre point, point type's origin, the key anchor). Each rule is a function of the app that returns what to draw;
//! `show` hands the results to the framework's controls, which keep what they stroke between passes.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use craft::behavior::Input;
use craft::behavior::Metrics;
use craft::draw::ui::{
    AnchorsShown, BezierOverlay, BezierTool, EditPath, EditSubpath, HandleStyle, HandlesShown, Knot, KnotRef, PathOptionsSettings, PathSelection,
    SegmentMark,
};
use craft::theme::Srgba;
use craft::ui::canvas::{CanvasSpace, canvas_space};
use craft::ui::canvas_controls::{HoverOutline, HoverShape, SelectionOutline};
use craft::ui::contract::{Access, Bind, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _};
use craft::ui::style::{ComponentStyle, DataColor, StyleCx};
use egui::{Color32, Rect, Response, Shape, Stroke, StrokeKind, vec2};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::{DocState, Prefs};
use vectorcraft_geom::{AnchorKind, BezPath, PathData, PathEl, Point};

use super::cursor::ViewKey;
use super::outline::{c32, node_outline, selection_hit_at, too_many_anchors, walk_drawn};
// PLAN: `BodyKind`, `Win` and `ViewKey::from(CanvasView)` are canvas-w10's (cv.S1, S2, S5); their paths here follow the contract.
use super::{BodyKind, VectorcraftApp, Win};
// PLAN: `anchors` takes `tool_points` because the Curvature tool's own points come from `session.overlays`, which only `show` may call (`&mut app`).
// PLAN: BezierOverlay selects anchors of every path under `HandlesShown`, so a whole-selected container draws handles that the old marks hid outside Direct Selection.

/// The anchor square's side at Selection & Anchor Display › Size 3, the default, in points.
const ANCHOR_AT_DEFAULT_SIZE: f32 = 5.0;
/// The default of Selection & Anchor Display › Size.
const DEFAULT_SIZE: u32 = 3;
/// How many points bigger than the other selected anchors the key anchor's square is drawn.
const KEY_ANCHOR_GROW: f32 = 2.0;
/// The gap between the key anchor's square and the frame drawn around it.
const KEY_ANCHOR_GAP: f32 = 2.0;
/// The radius of point type's origin dot, in points.
const ORIGIN_RADIUS: f32 = 2.5;

/// The outlines of the selected objects in one layer colour, as the `SelectionOutline` takes them.
pub(super) struct Outlined {
    /// The layer colour of every path in it.
    pub color: Color32,
    /// The outlines, in document units.
    pub paths: Vec<BezPath>,
    /// The key object's outline, drawn after the paths at the medium stroke.
    pub key: Option<BezPath>,
    /// [`outline_revision`] of `paths`, `key` and `color`.
    pub revision: u64,
}

/// What the anchors layer draws: the paths with their anchors, which are selected, and how they look.
pub(super) struct Anchors {
    /// The paths whose anchors show, keyed by node id.
    pub paths: Vec<EditPath>,
    /// The selected anchors and the selected segment.
    pub selection: PathSelection,
    /// Anchors as squares, or as circles under the Curvature tool.
    pub shown: AnchorsShown,
    /// Whose direction handles show.
    pub handles: HandlesShown,
    /// How a selected segment shows.
    pub segment_mark: SegmentMark,
    /// The user's anchor size and handle style.
    pub options: PathOptionsSettings,
}

/// The outline of the object under the pointer and its layer colour.
pub(super) struct Hovered {
    /// The object's outline, in document units.
    pub path: BezPath,
    /// The object's layer colour.
    pub color: Color32,
}

/// The dots of the selection: where each is, in document units, and its layer colour.
#[derive(Default)]
pub(super) struct Dots {
    /// Point type's origin.
    pub origins: Vec<(Point, Color32)>,
    /// The centre point of an object that shows it (Attributes › Show Center).
    pub centers: Vec<(Point, Color32)>,
    /// The key anchor (Align to Key Anchor).
    pub key_anchor: Option<(Point, Color32)>,
    /// Points bigger or smaller than the default anchor size.
    pub grow: f32,
}

/// A hash of what a `SelectionOutline` is given: every element of `paths` and of `key`, the element kinds, and
/// `extra` (the colour). It holds nothing alive, and two equal inputs give one value, so the framework may keep what
/// it stroked while the revision stays.
pub(super) fn outline_revision(paths: &[BezPath], key: Option<&BezPath>, extra: impl Hash) -> u64 {
    fn add(path: &BezPath, hasher: &mut DefaultHasher) {
        let mut point = |p: &Point| {
            p.x.to_bits().hash(hasher);
            p.y.to_bits().hash(hasher);
        };
        let mut kinds = Vec::with_capacity(path.elements().len());
        for el in path.elements() {
            match el {
                PathEl::MoveTo(a) => (kinds.push(0u8), point(a)),
                PathEl::LineTo(a) => (kinds.push(1), point(a)),
                PathEl::QuadTo(a, b) => (kinds.push(2), [a, b].into_iter().for_each(&mut point)),
                PathEl::CurveTo(a, b, c) => (kinds.push(3), [a, b, c].into_iter().for_each(&mut point)),
                PathEl::ClosePath => (kinds.push(4), ()),
            };
        }
        // The element kinds, so equal points under other kinds hash apart.
        kinds.hash(hasher);
    }
    let mut hasher = DefaultHasher::new();
    paths.len().hash(&mut hasher);
    paths.iter().for_each(|path| add(path, &mut hasher));
    key.is_some().hash(&mut hasher);
    if let Some(key) = key {
        add(key, &mut hasher);
    }
    extra.hash(&mut hasher);
    hasher.finish()
}

/// The Path Options the anchors layer draws with: Selection & Anchor Display › Size (1–7, one point per step) and
/// Handles. One builder, so every overlay reads the preference the same way.
#[allow(clippy::field_reassign_with_default)] // `PathOptionsSettings` is non-exhaustive: no struct literal outside its crate
pub(super) fn path_options(prefs: &Prefs) -> PathOptionsSettings {
    let mut options = PathOptionsSettings::default();
    options.anchor_size = Some(ANCHOR_AT_DEFAULT_SIZE + grow(prefs));
    options.handles = match prefs.handle_style.as_str() {
        "hollow" => HandleStyle::Hollow,
        "large" => HandleStyle::Large,
        _ => HandleStyle::Solid,
    };
    options
}

/// Points bigger (or smaller) than the default size, per step of Size.
fn grow(prefs: &Prefs) -> f32 {
    prefs.anchor_size.clamp(1, 7) as f32 - DEFAULT_SIZE as f32
}

/// The mark rules do not apply: Halftone Preview selects its dots, whose marks would hide the art.
fn hidden(app: &VectorcraftApp) -> bool {
    crate::dialogs::halftone::previewing(app)
}

/// Whether the selection has too many points to outline: its bounds stand for it.
fn too_big(st: &DocState) -> bool {
    too_many_anchors(st.selection.objects.iter().filter_map(|id| st.doc.node(*id)))
}

/// The rectangle of `bounds` as a path.
fn bounds_path(bounds: vectorcraft_geom::Rect) -> BezPath {
    vectorcraft_geom::shapes::rectangle(bounds).to_bezpath()
}

/// The group of `out` in layer colour `color`, added when there is none.
fn group(out: &mut Vec<Outlined>, color: Color32) -> &mut Outlined {
    let at = out.iter().position(|o| o.color == color).unwrap_or_else(|| {
        out.push(Outlined { color, paths: vec![], key: None, revision: 0 });
        out.len() - 1
    });
    &mut out[at]
}

/// The outlines of the selection, one [`Outlined`] per layer colour: each selected object's outline (the bounds of
/// all of them when they have too many points), the spine of each selected blend, and the key object's outline (its
/// bounds when it has too many points). Nothing during Halftone Preview.
pub(super) fn outlines(app: &VectorcraftApp, st: &DocState) -> Vec<Outlined> {
    let mut out: Vec<Outlined> = vec![];
    if hidden(app) {
        return out;
    }
    let (sel, symbols) = (&st.selection, st.doc.symbol_boxes());
    let color_of = |id: NodeId| c32(st.doc.layer_color(id));
    if too_big(st) {
        // The cache is warmed by `show`; a test or a miss measures again.
        let bounds = match app.canvas.selection_bounds {
            Some((key, bounds)) if key == (st.uid, st.revision) => bounds,
            _ => st.doc.bounds_of(&sel.objects, true),
        };
        if let Some((bounds, id)) = bounds.zip(sel.objects.first()) {
            group(&mut out, color_of(*id)).paths.push(bounds_path(bounds));
        }
    } else {
        for id in &sel.objects {
            let Some(node) = st.doc.node(*id) else { continue };
            let path = node_outline(node, &symbols);
            if !path.elements().is_empty() {
                group(&mut out, color_of(*id)).paths.push(path);
            }
        }
        for (blend, spine) in blend_spines(st) {
            group(&mut out, color_of(blend)).paths.push(spine.to_bezpath());
        }
    }
    if let Some((key, node)) = sel.key.and_then(|key| Some((key, st.doc.node(key)?))) {
        let path = if too_many_anchors([node]) { node.geometric_bounds_with(&symbols).map(bounds_path) } else { Some(node_outline(node, &symbols)) };
        if let Some(path) = path {
            group(&mut out, color_of(key)).key = Some(path);
        }
    }
    for o in &mut out {
        o.revision = outline_revision(&o.paths, o.key.as_ref(), o.color.to_array());
    }
    out
}

/// The spine of each selected blend, or of the blend a selected key object belongs to, with the blend's id.
fn blend_spines(st: &DocState) -> Vec<(NodeId, PathData)> {
    let is_blend = |id: &NodeId| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Blend { .. }));
    let mut seen: Vec<NodeId> = vec![];
    let mut spines = vec![];
    for id in &st.selection.objects {
        let Some(blend) = [Some(*id), st.doc.parent_of(*id)].into_iter().flatten().find(is_blend).filter(|b| !seen.contains(b)) else { continue };
        seen.push(blend);
        let Some(NodeKind::Blend { children, spec }) = st.doc.node(blend).map(|n| &n.kind) else { continue };
        if let Some((path, _)) = vectorcraft_doc::live::blend_spine(children, spec) {
            spines.push((blend, path));
        }
    }
    spines
}

/// `color` as the framework's colour.
fn srgba(color: Color32) -> Srgba {
    let channel = |c: u8| f32::from(c) / 255.0;
    Srgba::rgb(channel(color.r()), channel(color.g()), channel(color.b()))
}

/// `path` as a path the anchors layer edits, keyed `key`, in `color`.
fn edit_path(key: u64, path: &PathData, color: Color32) -> EditPath {
    let subpaths = path
        .subpaths
        .iter()
        .map(|sp| {
            let knots = sp.anchors.iter().map(|a| Knot::new(a.p, a.h_in, a.h_out, a.kind == AnchorKind::Smooth)).collect();
            EditSubpath::new(knots, sp.closed)
        })
        .collect();
    EditPath::new(key, subpaths).colored(Some(srgba(color)))
}

/// The points of a gradient mesh or an envelope's mesh, each an open subpath of one corner anchor.
fn grid_path(key: u64, points: impl Iterator<Item = Point>, color: Color32) -> EditPath {
    let subpaths = points.map(|p| EditSubpath::new(vec![Knot::corner(p)], false)).collect();
    EditPath::new(key, subpaths).colored(Some(srgba(color)))
}

/// The anchors and handles of the selection. An object selected whole has every anchor selected, and so do the
/// paths inside a group, compound path or clip group selected whole; with Direct Selection's partial selection only
/// the chosen anchors are, and a selected segment hollows the others. `tool_points` is whether the tool draws its own
/// points (the Curvature tool while it draws), which leaves nothing here. Nothing too big to outline, and nothing
/// during Halftone Preview.
pub(super) fn anchors(app: &VectorcraftApp, st: &DocState, tool_points: bool) -> Anchors {
    let tool = app.session.tool_id();
    let curvature = tool == "curvature";
    let prefs = &app.session.prefs;
    let mut out = Anchors {
        paths: vec![],
        selection: PathSelection::default(),
        shown: if curvature { AnchorsShown::Curvature } else { AnchorsShown::All },
        handles: if prefs.show_handles_multiple_anchors { HandlesShown::Selected } else { HandlesShown::OneAnchor },
        segment_mark: SegmentMark::Line,
        options: path_options(prefs),
    };
    if hidden(app) || too_big(st) || (curvature && tool_points) {
        return out;
    }
    let sel = &st.selection;
    // The selected anchors by path, and the selected segment with its path.
    let mut chosen: Vec<(u64, Vec<KnotRef>)> = vec![];
    let mut segment: Option<(u64, KnotRef)> = None;
    for id in &sel.objects {
        let Some(node) = st.doc.node(*id) else { continue };
        let partial = sel.partial(*id);
        let color = c32(st.doc.layer_color(*id));
        walk_drawn(node, &mut |c| {
            let envelope = vectorcraft_doc::live::envelope_overlay(c).and_then(|(_, grid)| grid);
            let grid = match &c.kind {
                NodeKind::Mesh(m) => Some(m),
                _ => envelope.as_ref(),
            };
            if let Some(grid) = grid {
                out.paths.push(grid_path(c.id.0, grid.points.iter().map(|q| q.p), color));
                return;
            }
            let NodeKind::Path { path, .. } = &c.kind else { return };
            out.paths.push(edit_path(c.id.0, path, color));
            let segments = sel.segments.get(&c.id);
            if let Some(&(si, seg)) = segments.and_then(|s| s.iter().next()) {
                segment.get_or_insert((c.id.0, KnotRef::new(si, seg)));
            }
            // A selected segment shows the others' anchors hollow.
            if segments.is_some() {
                return;
            }
            let knots: Vec<KnotRef> = path
                .anchors()
                .filter(|(si, ai, _)| partial.is_none_or(|set| set.contains(&(*si, *ai))))
                .map(|(si, ai, _)| KnotRef::new(si, ai))
                .collect();
            if !knots.is_empty() {
                chosen.push((c.id.0, knots));
            }
        });
    }
    for (key, spine) in blend_spines(st) {
        out.paths.push(edit_path(key.0, &spine, c32(st.doc.layer_color(key))));
    }
    // The path the overlay edits is the segment's, else the first with selected anchors; the rest draw selected.
    let primary = segment.map(|(key, _)| key).or_else(|| chosen.first().map(|(key, _)| *key));
    let mut selection = PathSelection::new(
        primary,
        primary.and_then(|key| chosen.iter().find(|(k, _)| *k == key)).map(|(_, knots)| knots.clone()).unwrap_or_default(),
        segment.map(|(_, at)| at),
    );
    for (key, knots) in chosen.into_iter().filter(|(key, _)| Some(*key) != primary) {
        selection = selection.also(key, knots);
    }
    out.selection = selection;
    out
}

/// The outline of the object under the pointer at document point `at`, at view `zoom`, for Smart Guides › Object
/// Highlighting: the object a click of the selection tools would pick, unless it is selected or has too many points.
/// None while the view moves (`still` false), under Snap to Grid or Pixel Preview, with a tool that is busy or is not
/// a selection tool, and with the preference off.
pub(super) fn hover(app: &VectorcraftApp, st: &DocState, at: Point, zoom: f64, still: bool) -> Option<Hovered> {
    let (view, prefs) = (&app.ui.view, &app.session.prefs);
    if !still || !view.smart_guides || !prefs.object_highlighting || view.snap_to_grid || view.pixel_preview {
        return None;
    }
    let tool = app.session.tool_id();
    if app.session.tool_busy() || !matches!(tool, "selection" | "directSelection" | "groupSelection") {
        return None;
    }
    let hit = selection_hit_at(app, at, zoom)?;
    let id = if tool == "selection" { hit.top_object(st.isolation) } else { hit.leaf };
    if st.selection.contains(id) {
        return None;
    }
    let node = st.doc.node(id).filter(|n| !too_many_anchors([*n]))?;
    Some(Hovered { path: node_outline(node, &st.doc.symbol_boxes()), color: c32(st.doc.layer_color(id)) })
}

/// The dots of the selection: point type's origin, the centre of each object that shows it, and the key anchor.
pub(super) fn dots(app: &VectorcraftApp, st: &DocState) -> Dots {
    let mut out = Dots { grow: grow(&app.session.prefs), ..Dots::default() };
    if hidden(app) || too_big(st) {
        return out;
    }
    let symbols = st.doc.symbol_boxes();
    for id in &st.selection.objects {
        let Some(node) = st.doc.node(*id) else { continue };
        let color = c32(st.doc.layer_color(*id));
        if node.shows_center()
            && let Some(b) = node.geometric_bounds_with(&symbols)
        {
            out.centers.push((b.center(), color));
        }
        if let NodeKind::Text(tx) = &node.kind
            && matches!(tx.kind, vectorcraft_doc::TextKind::Point)
        {
            out.origins.push((tx.xf * Point::ZERO, color));
        }
    }
    if let Some(((id, _), at)) = st.selection.key_anchor.zip(st.selection.key_anchor_point(&st.doc)) {
        out.key_anchor = Some((at, c32(st.doc.layer_color(id))));
    }
    out
}

/// The look of [`SelectionDots`]: the anchor square's side and the medium line.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct DotsStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The side of a selected anchor's square.
    side: f32,
    /// The width of the key anchor's frame.
    medium: f32,
}

/// The dots of the selection, painted over the canvas: no pointer, key or accessibility node.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control, content_painter)]
struct SelectionDots<'a> {
    #[common]
    common: Common,
    dots: &'a Dots,
}

impl Component for SelectionDots<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = DotsStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: SelectionDotsStyleKey) -> DotsStyle {
        DotsStyle { metrics: Metrics::new(), side: cx.tokens.size.handle.sm, medium: cx.tokens.stroke.medium }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let (view, dots, style) = (space.view, self.dots, &cx.style);
        let painter = ui.content_painter(cx);
        let square = |at: Point, side: f32| Rect::from_center_size(view.to_screen(at), vec2(side, side));
        let side = style.side + dots.grow;
        for &(at, color) in &dots.centers {
            painter.painter.add(Shape::rect_filled(square(at, side), 0.0, color));
        }
        for &(at, color) in &dots.origins {
            painter.painter.add(Shape::circle_filled(view.to_screen(at), ORIGIN_RADIUS, color));
        }
        // The key anchor: a filled square bigger than the other anchors, in a frame as thick as a key object's outline.
        if let Some((at, color)) = dots.key_anchor {
            let key = side + KEY_ANCHOR_GROW;
            painter.painter.add(Shape::rect_filled(square(at, key), 0.0, color));
            let frame = square(at, key + 2.0 * KEY_ANCHOR_GAP);
            painter.painter.add(Shape::rect_stroke(frame, 0.0, Stroke::new(style.medium, color), StrokeKind::Outside));
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Whether the active tool draws its own points over the paths: the Curvature tool while it has a draft.
fn tool_draws_points(app: &mut VectorcraftApp) -> bool {
    if app.session.tool_id() != "curvature" {
        return false;
    }
    let view = app.view_info();
    app.session.overlays(view).iter().any(|o| matches!(o, vectorcraft_tools::Overlay::CurvePoint { .. }))
}

/// Shows the selection's marks in the canvas body, in the order the old canvas drew them: the hover outline, the
/// outlines, the anchors, then the dots. They show only with the edges on and outside a Revolve edit. A `Pane` body is
/// display-only, so it shows no hover.
pub(super) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, space: &CanvasSpace, _win: Win, kind: BodyKind) {
    if !app.ui.view.edges || super::revolve_editing(app) {
        return;
    }
    let rect = space.view.rect;
    // Measured once per revision and kept; `outlines` reads it.
    app.selection_bounds();
    let tool_points = tool_draws_points(app);
    let still = {
        let wait = super::cursor::hover_wait(&mut app.canvas.view_moved, ViewKey::of(&space.view), craft::ui::runtime::frame::clock(ui.ctx()));
        if let Some(secs) = wait {
            ui.ctx().request_repaint_after_secs(secs as f32);
        }
        wait.is_none()
    };
    let Some(st) = app.session.active() else { return };
    let hovered = match (kind, app.hover_doc) {
        (BodyKind::Pane, _) | (_, None) => None,
        (_, Some(at)) => hover(app, st, at, space.view.zoom, still),
    };
    if let Some(h) = hovered {
        HoverOutline::new("Object highlight", Some(HoverShape::Path(h.path))).color(DataColor::new(h.color)).show_at(ui, rect);
    }
    for o in outlines(app, st) {
        let mut outline = SelectionOutline::solid("Selection", &o.paths).color(DataColor::new(o.color)).revision(o.revision);
        if let Some(key) = &o.key {
            outline = outline.key(key);
        }
        outline.show_at(ui, rect);
    }
    let marks = anchors(app, st, tool_points);
    if !marks.paths.is_empty() {
        let mut paths = marks.paths;
        // The marks only show the selection: the tools make the edits.
        let _ = BezierOverlay::new("Anchors", Bind::bound(&mut paths), &marks.selection)
            .tool(BezierTool::Anchors)
            .handles(marks.handles)
            .anchors(marks.shown)
            .segment_mark(marks.segment_mark)
            .path_options(&marks.options)
            .show_at(ui, rect);
    }
    let dots = dots(app, st);
    if !(dots.centers.is_empty() && dots.origins.is_empty() && dots.key_anchor.is_none()) {
        SelectionDots { common: Common::new(), dots: &dots }.label("Selection dots").show_at(ui, rect);
    }
}

#[cfg(test)]
mod tests;
