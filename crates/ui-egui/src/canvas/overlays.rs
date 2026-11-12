use super::artboards::names_an_artboard;
use super::input::marquee_through;
use super::outline::{KEY_OUTLINE, c32, node_outline, selection_hit_at, stroke_path, to_screen_path, too_many_anchors};
use super::selection_marks::{MarkOptions, SelectionMarks};
use super::{Color32, CornerRadius, NodeKind, Overlay, Pos2, Shape, Stroke, StrokeKind, Tokens, VectorcraftApp, Xf, vec2};

pub(super) fn hover_highlight(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(h) = app.hover_doc else { return };
    // Smart Guides › Object Highlighting: a Smart Guides display option, so it needs them on and Snap to Grid and Pixel Preview off.
    if !app.ui.view.smart_guides || !app.session.prefs.object_highlighting || app.ui.view.snap_to_grid || app.ui.view.pixel_preview {
        return;
    }
    if app.session.tool_busy() || !matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection") {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(hit) = selection_hit_at(app, h, xf.zoom) else { return };
    let id = if app.session.tool_id() == "selection" { hit.top_object(st.isolation) } else { hit.leaf };
    if st.selection.contains(id) {
        return;
    }
    if let Some(n) = st.doc.node(id).filter(|n| !too_many_anchors([*n])) {
        let color = c32(st.doc.layer_color(id));
        stroke_path(p, &node_outline(n, &st.doc.symbol_boxes()), xf, Stroke::new(1.5, color));
    }
}

/// Draw Inside (Shift+D): while the mode lasts, a dotted open rectangle round the corners of the
/// object drawn into, selected or not (#4319).
pub(super) fn draw_inside_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    if app.session.draw_mode != vectorcraft_engine::DrawMode::Inside {
        return;
    }
    let (Some(id), Some(st)) = (app.session.draw_inside, app.session.active()) else { return };
    let Some(bounds) = st.doc.node(id).and_then(|n| n.visual_bounds()) else { return };
    // A few pixels outside the object, clear of its stroke.
    let pad = 4.0 / xf.zoom;
    let [a, b, c, d] = xf.quad(bounds.inflate(pad, pad))[..] else { return };
    let stroke = Stroke::new(1.0, c32(st.doc.layer_color(id)));
    for (corner, sides) in [(a, [b, d]), (b, [c, a]), (c, [d, b]), (d, [a, c])] {
        for next in sides {
            // Each arm runs a third of its side at most, so the rectangle stays open.
            let arm = ((next - corner).length() / 3.0).min(16.0);
            p.extend(Shape::dashed_line(&[corner, corner + (next - corner).normalized() * arm], stroke, 3.0, 2.0));
        }
    }
}

/// Selected anchors are drawn slightly deeper than the layer colour (#4f80ff → #3d82ff for Layer 1).
pub(super) fn selected_anchor(c: Color32) -> Color32 {
    if c == Color32::from_rgb(0x4f, 0x80, 0xff) { Color32::from_rgb(0x3d, 0x82, 0xff) } else { c }
}

/// How anchors and handles look: Selection & Anchor Display › Size and Handles.
#[derive(Clone, Copy)]
pub(super) struct HandleLook<'a> {
    /// Points bigger (or smaller) than the default size 3, per step of Size (1–7).
    pub(super) grow: f32,
    /// `solid` (the default), `hollow` or `large`.
    pub(super) style: &'a str,
}

impl<'a> HandleLook<'a> {
    pub(super) fn of(prefs: &'a vectorcraft_engine::Prefs) -> Self {
        Self { grow: prefs.anchor_size.clamp(1, 7) as f32 - 3.0, style: &prefs.handle_style }
    }

    /// A direction handle's end at `c`: a solid dot, a hollow one or a larger solid one.
    pub(super) fn shapes(self, c: Pos2, color: Color32) -> [Option<Shape>; 2] {
        let r = 2.75 + self.grow / 2.0;
        let hollow = self.style == "hollow";
        let dot = Shape::circle_filled(c, if self.style == "large" { r + 1.5 } else { r }, if hollow { Color32::WHITE } else { color });
        [Some(dot), hollow.then(|| Shape::circle_stroke(c, r, Stroke::new(1.0, color)))]
    }

    pub(super) fn draw(self, p: &egui::Painter, c: Pos2, color: Color32) {
        for shape in self.shapes(c, color).into_iter().flatten() {
            p.add(shape);
        }
    }
}

pub(super) fn anchor_square(p: &egui::Painter, c: Pos2, color: Color32, filled: bool, size: f32) {
    let r = egui::Rect::from_center_size(c, vec2(size, size));
    if filled {
        p.rect_filled(r, 0.0, color);
    } else {
        p.rect_filled(r, 0.0, Color32::WHITE);
        p.rect_stroke(r, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
    }
}

pub(super) fn curve_point(p: &egui::Painter, c: Pos2, color: Color32, filled: bool, size: f32) {
    p.circle(c, size / 2.0, if filled { color } else { Color32::WHITE }, Stroke::new(1.0, color));
}

/// How the current tool and preferences draw the selection's marks.
pub(super) fn mark_options(app: &VectorcraftApp, curve_points_from_tool: bool) -> MarkOptions<'_> {
    let tool = app.session.tool_id();
    MarkOptions {
        direct: vectorcraft_tools::catalog::edits_anchors(tool),
        curvature: tool == "curvature",
        curve_points_from_tool,
        look: HandleLook::of(&app.session.prefs),
        all_handles: app.session.prefs.show_handles_multiple_anchors,
    }
}

/// Draw selection marks and widgets, keeping the box but hiding dot marks during Halftone Preview.
pub(super) fn selection_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf, curve_points_from_tool: bool, marks: &mut SelectionMarks) {
    // The Selection tool's bounding box (rotated with the objects after a rotation), also shown by
    // the shape tools that keep a shape just drawn ready to adjust while they draw none (#5477). It
    // hides while a drag moves or resizes the selection, so only the art is seen going with the
    // pointer (#712). A handle resize keeps it at the changing bounds (#3526).
    // With Real-time Drawing off the art waits for the release of a move drag, and the box moves
    // with the pointer instead.
    let resizing = app.session.tool_resizing();
    let moved = app.session.deferred_move();
    let shape_tool = vectorcraft_tools::shape::shows_shape_widgets(app.session.tool_id());
    // A shape tool drawing a shape shows none of this; adjusting a widget, it shows the widgets.
    let drawing = shape_tool && app.session.tool_busy() && !app.session.tool_transforming();
    // Cmd held lends the Selection tool to a press (Direct Selection's included, #3901): its box
    // shows while the key is down, where its handles take that press.
    let held = super::drag::mods(p.ctx().input(|i| i.modifiers), false);
    let lends_selection = !app.session.tool_busy() && app.session.lent_tool_id(&held) == Some("selection");
    let show_box = (app.session.tool_id() == "selection" || lends_selection || (shape_tool && (!app.session.tool_busy() || resizing)))
        && (resizing || !app.session.tool_transforming() || moved.is_some())
        && app.ui.view.bounding_box
        && app.session.active().is_some_and(|st| !st.selection.is_empty() && st.selection.anchors.is_empty());
    // With Real-time Drawing off the document stays at the drag's snapshot, so a resize measures the
    // box from the pending preview; otherwise each step's revision measures it once (`selection_box`).
    let bbox = if !show_box {
        None
    } else if let Some(b) = resizing.then(|| app.session.deferred_transform_box()).flatten() {
        Some(b)
    } else {
        app.selection_box()
    };
    let bbox = bbox.map(|b| match moved {
        Some(d) => vectorcraft_doc::OrientedBox { rect: b.rect + b.to_local(d.to_point()).to_vec2(), ..b },
        None => b,
    });
    let st = app.session.active();
    let show_marks = !crate::dialogs::halftone::previewing(app);
    let big = show_marks && st.is_some_and(|st| too_many_anchors(st.selection.objects.iter().filter_map(|id| st.doc.node(*id))));
    let big_bounds = if big { app.selection_bounds() } else { None };
    let app = &*app;
    let Some(st) = app.session.active() else { return };
    let symbols = st.doc.symbol_boxes();
    let tool = app.session.tool_id();
    let direct = vectorcraft_tools::catalog::edits_anchors(tool);
    // Selection & Anchor Display › Size (1–7, 3 the default): anchors, handles and the bounding
    // box's handles a point bigger or smaller per step.
    let options = mark_options(app, curve_points_from_tool);
    let grow = options.look.grow;
    let anchor = |direct: bool| grow + if direct { 5.0 } else { 4.0 };
    // The temporary halftone selects its dots; their marks obscure the art (#6369).
    if show_marks {
        // Too many paths to outline: the selection's bounds stand for them.
        if let Some((b, id)) = big_bounds.zip(st.selection.objects.first()) {
            stroke_path(p, &vectorcraft_geom::shapes::rectangle(b).to_bezpath(), xf, Stroke::new(1.0, c32(st.doc.layer_color(*id))));
        }
        // The key object (Align to Key Object): its outline drawn thicker, or its bounds when it has
        // too many paths to outline.
        let key = st.selection.key.and_then(|k| Some((k, st.doc.node(k)?)));
        let (key_bounds, key_outline) = match key {
            Some((k, n)) if too_many_anchors([n]) => (Some((k, n)), None),
            Some((k, _)) => (None, Some(k)),
            None => (None, None),
        };
        if let Some((k, n)) = key_bounds
            && let Some(b) = n.geometric_bounds_with(&symbols)
        {
            stroke_path(p, &vectorcraft_geom::shapes::rectangle(b).to_bezpath(), xf, Stroke::new(KEY_OUTLINE, c32(st.doc.layer_color(k))));
        }
        marks.paint(st, p, xf, &options, key_outline, if big { &[] } else { &st.selection.objects });
    }
    // The spine of each selected blend (or of the blend a selected key object belongs to).
    let mut spines = vec![];
    for id in &st.selection.objects {
        let is_blend = |b: &vectorcraft_doc::NodeId| st.doc.node(*b).is_some_and(|n| matches!(n.kind, NodeKind::Blend { .. }));
        let Some(b) = [Some(*id), st.doc.parent_of(*id)].into_iter().flatten().find(is_blend).filter(|b| !spines.contains(b)) else { continue };
        spines.push(b);
        let Some(NodeKind::Blend { children, spec }) = st.doc.node(b).map(|n| &n.kind) else { continue };
        let Some((path, _)) = vectorcraft_doc::live::blend_spine(children, spec) else { continue };
        let color = c32(st.doc.layer_color(b));
        stroke_path(p, &path.to_bezpath(), xf, Stroke::new(1.0, color));
        for (_, _, a) in path.anchors() {
            anchor_square(p, xf.to_screen(a.p), color, false, anchor(direct));
        }
    }
    // Live Corners widgets (Selection and the shape tools on a live rectangle or polygon, Direct
    // Selection on any path).
    if (matches!(tool, "selection" | "directSelection") || (shape_tool && !drawing))
        && let Some(w) = vectorcraft_tools::corners::CornerWidgets::showing(
            &st.doc,
            &st.selection,
            xf.zoom,
            tool == "directSelection",
            app.ui.view.corner_widgets,
            app.session.prefs.hide_corner_widget_above,
        )
    {
        for (id, q) in w.visible_on() {
            let (sp, color) = (xf.to_screen(q), c32(st.doc.layer_color(id)));
            p.circle_filled(sp, 3.0, Color32::WHITE);
            p.circle_stroke(sp, 3.0, Stroke::new(1.0, color));
            p.circle_filled(sp, 1.0, color);
        }
    }
    // Bounding box with handles (Selection tool).
    if let Some(b) = bbox {
        let color = c32(st.doc.layer_color(st.selection.objects[0]));
        p.add(Shape::closed_line(b.corners().iter().map(|q| xf.to_screen(*q)).collect(), Stroke::new(1.0, color)));
        for h in vectorcraft_tools::bbox::Handle::ALL {
            let c = xf.to_screen(b.to_doc() * h.pos(b.rect));
            let hr = egui::Rect::from_center_size(c, vec2(6.0 + grow, 6.0 + grow));
            p.rect_filled(hr, 0.0, Color32::WHITE);
            p.rect_stroke(hr, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
        }
        // The type widget beside it: hollow on point type, filled on area type.
        if let Some(w) = vectorcraft_tools::typewidget::TypeWidget::of(&st.doc, &st.selection, &b, xf.zoom) {
            let (c, r) = (xf.to_screen(w.at), vectorcraft_tools::typewidget::RADIUS_PX);
            p.circle_filled(c, r, if w.area { color } else { Color32::WHITE });
            p.circle_stroke(c, r, Stroke::new(1.0, color));
        }
    }
    // Pie widgets (Selection tool, with View › Show Bounding Box, on live ellipses): each joined
    // by a line to the outline's point at its angle. They stay while one is dragged.
    if (tool == "selection" || (shape_tool && !drawing)) && app.ui.view.bounding_box {
        for w in vectorcraft_tools::pie::PieWidget::of(&st.doc, &st.selection, xf.zoom) {
            let (on, c, r, color) = (xf.to_screen(w.on), xf.to_screen(w.at), vectorcraft_tools::pie::RADIUS_PX, c32(st.doc.layer_color(w.id)));
            p.line_segment([on, c], Stroke::new(1.0, color));
            p.circle_filled(c, r, Color32::WHITE);
            p.circle_stroke(c, r, Stroke::new(1.0, color));
        }
    }
    rotate_all_widget(p, xf, tool, &st.doc, &st.selection);
}

/// The Select/Move, Rotate All, Space and Move All widgets (Selection and Objects on Path tools, one
/// Objects on Path group selected). The Rotate All widget is joined by a line to the first object's
/// attachment point. It stays while dragged.
fn rotate_all_widget(p: &egui::Painter, xf: &Xf, tool: &str, doc: &vectorcraft_doc::Document, selection: &vectorcraft_doc::Selection) {
    use vectorcraft_tools::objects_on_path::{ROTATE_ALL_RADIUS_PX, RotateAllWidget};
    if !matches!(tool, "selection" | "objectsOnPath") {
        return;
    }
    member_widgets(p, xf, doc, selection);
    if let Some(w) = RotateAllWidget::of(doc, selection, xf.zoom) {
        let (on, c, color) = (xf.to_screen(w.pivot), xf.to_screen(w.at), c32(doc.layer_color(w.group)));
        p.line_segment([on, c], Stroke::new(1.0, color));
        p.circle_filled(c, ROTATE_ALL_RADIUS_PX, Color32::WHITE);
        p.circle_stroke(c, ROTATE_ALL_RADIUS_PX, Stroke::new(1.0, color));
    }
    space_widget(p, xf, doc, selection);
    move_all_widget(p, xf, doc, selection);
}

/// The Select/Move widgets: a ring on each object's attachment point, filled blue for the chosen
/// object. They are drawn under the other widgets, which a press takes first.
fn member_widgets(p: &egui::Painter, xf: &Xf, doc: &vectorcraft_doc::Document, selection: &vectorcraft_doc::Selection) {
    use vectorcraft_tools::objects_on_path::{MEMBER_RADIUS_PX, MemberWidget};
    for w in MemberWidget::all(doc, selection, xf.zoom) {
        let c = xf.to_screen(w.at);
        if w.chosen {
            p.circle(c, MEMBER_RADIUS_PX, MEMBER_CHOSEN, Stroke::new(1.0, Color32::WHITE));
        } else {
            p.circle(c, MEMBER_RADIUS_PX, Color32::TRANSPARENT, Stroke::new(1.5, c32(doc.layer_color(w.group))));
        }
    }
}

/// The fill of the chosen object's Select/Move widget.
pub(super) const MEMBER_CHOSEN: Color32 = Color32::from_rgb(0x1f, 0x6f, 0xeb);

/// The Space widget (beside the Rotate All widget, with two or more objects on the path): a square
/// joined by a line to the last object's attachment point. It stays while dragged.
fn space_widget(p: &egui::Painter, xf: &Xf, doc: &vectorcraft_doc::Document, selection: &vectorcraft_doc::Selection) {
    use vectorcraft_tools::objects_on_path::{SPACE_HALF_PX, SpaceWidget};
    let Some(w) = SpaceWidget::of(doc, selection, xf.zoom) else { return };
    let (on, c, color) = (xf.to_screen(w.last), xf.to_screen(w.at), c32(doc.layer_color(w.group)));
    let r = egui::Rect::from_center_size(c, vec2(2.0 * SPACE_HALF_PX, 2.0 * SPACE_HALF_PX));
    p.line_segment([on, c], Stroke::new(1.0, color));
    p.rect_filled(r, 0.0, Color32::WHITE);
    p.rect_stroke(r, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
}

/// The Move All widget (beside the Rotate All widget, when the objects have room to move along the
/// path): a diamond below the first object joined by a line to its attachment point. It stays while
/// dragged.
fn move_all_widget(p: &egui::Painter, xf: &Xf, doc: &vectorcraft_doc::Document, selection: &vectorcraft_doc::Selection) {
    use vectorcraft_tools::objects_on_path::{MOVE_ALL_HALF_PX, MoveAllWidget};
    let Some(w) = MoveAllWidget::of(doc, selection, xf.zoom) else { return };
    let (on, c, color) = (xf.to_screen(w.first), xf.to_screen(w.at), c32(doc.layer_color(w.group)));
    let h = MOVE_ALL_HALF_PX;
    p.line_segment([on, c], Stroke::new(1.0, color));
    let corners = vec![c + vec2(0.0, -h), c + vec2(h, 0.0), c + vec2(0.0, h), c + vec2(-h, 0.0)];
    p.add(Shape::convex_polygon(corners, Color32::WHITE, Stroke::new(1.0, color)));
}

/// Record where the Type tool's IME candidate window goes this pass, if it has an editor.
///
/// The descriptor itself is observed once at the end of the pass (`crate::ime::end_of_pass`),
/// after dialogs, floating panels, progress windows and the palette draw: any of them can open
/// over the canvas, take the keyboard or run a command that changes the edited text, so a
/// descriptor written here would carry a stale owner, surrounding text or cause.
pub(super) fn ime_geometry(app: &mut VectorcraftApp, ctx: &egui::Context, xf: &Xf) {
    app.ime_caret_rect = None;
    // A hidden viewport cannot compose; the end-of-pass observation clears the owner.
    if !app.session.tool_wants_text() || app.viewport_hidden(ctx) {
        return;
    }
    let Some((a, b)) = app.session.tool_ime_caret(app.view_info()) else { return };
    let r = egui::Rect::from_two_pos(xf.to_screen(a), xf.to_screen(b)).expand2(vec2(1.0, 0.0));
    app.ime_caret_rect = Some((r, r));
}

pub(super) fn draw_overlays(p: &egui::Painter, xf: &Xf, overlays: &[Overlay], t: &Tokens, look: HandleLook) {
    for o in overlays {
        match o {
            // Where the tools select: the document rectangle, turned with the view.
            Overlay::Marquee(r) => marquee_through(p, &xf.quad(*r)),
            Overlay::SelectionMarquee { rect, enclosed, pointer } => {
                let quad = xf.quad(*rect);
                if *enclosed {
                    let points: Vec<Pos2> = quad.iter().chain(quad.first()).copied().collect();
                    p.add(Shape::line(points.clone(), Stroke::new(3.0, Color32::from_white_alpha(230))));
                    p.add(Shape::line(points, Stroke::new(1.0, Color32::from_gray(40))));
                } else {
                    marquee_through(p, &quad);
                }
                let (current, other) = if *enclosed { (tl!("Fully Enclosed"), tl!("Touching")) } else { (tl!("Touching"), tl!("Fully Enclosed")) };
                let label = format!("{current} · E: {other}");
                let galley = p.layout(label, egui::FontId::proportional(11.0), t.text, (p.clip_rect().width() - 12.0).max(1.0));
                let size = galley.size() + vec2(12.0, 8.0);
                let wanted = xf.to_screen(*pointer) + vec2(14.0, 14.0);
                let clip = p.clip_rect();
                let min = egui::pos2(
                    wanted.x.clamp(clip.min.x, (clip.max.x - size.x).max(clip.min.x)),
                    wanted.y.clamp(clip.min.y, (clip.max.y - size.y).max(clip.min.y)),
                );
                let badge = egui::Rect::from_min_size(min, size);
                p.rect_filled(badge, CornerRadius::same(3), t.panel);
                p.galley(badge.min + vec2(6.0, 4.0), galley, t.text);
            }
            Overlay::Path { path, color, width, dashed } => {
                let s = Stroke::new(*width, c32(*color));
                if *dashed {
                    for line in to_screen_path(path, xf) {
                        p.extend(Shape::dashed_line(&line, s, 4.0, 3.0));
                    }
                } else {
                    stroke_path(p, path, xf, s);
                }
            }
            Overlay::Line { a, b, color, dashed } => {
                let s = Stroke::new(1.0, c32(*color));
                let (a, b) = (xf.to_screen(*a), xf.to_screen(*b));
                if *dashed {
                    p.extend(Shape::dashed_line(&[a, b], s, 4.0, 3.0));
                } else {
                    p.line_segment([a, b], s);
                }
            }
            Overlay::Anchor { p: pt, color, filled, size } => anchor_square(p, xf.to_screen(*pt), c32(*color), *filled, *size),
            Overlay::CurvePoint { p: pt, color, filled, size } => curve_point(p, xf.to_screen(*pt), c32(*color), *filled, *size + look.grow),
            Overlay::Handle { p: pt, color } => look.draw(p, xf.to_screen(*pt), c32(*color)),
            Overlay::Label { p: pt, text, color } => {
                let sp = xf.to_screen(*pt) + vec2(8.0, -14.0);
                p.text(
                    sp,
                    egui::Align2::LEFT_TOP,
                    crate::panels::label_or_name(text, !names_an_artboard(text)),
                    egui::FontId::proportional(11.0),
                    c32(*color),
                );
            }
            Overlay::Highlight { quad, color } => {
                let c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                p.add(Shape::convex_polygon(quad.iter().map(|q| xf.to_screen(*q)).collect(), c, Stroke::NONE));
            }
            Overlay::Measure { p: pt, text } => {
                let sp = xf.to_screen(*pt) + vec2(14.0, 14.0);
                let galley = p.layout(text.clone(), egui::FontId::proportional(11.0), Color32::WHITE, 200.0);
                let r = egui::Rect::from_min_size(sp, galley.size() + vec2(12.0, 8.0));
                p.rect_filled(r, t.cr(), t.measure_bg);
                p.galley(sp + vec2(6.0, 4.0), galley, Color32::WHITE);
            }
            Overlay::Distance { a, b, text, color } => distance_guide(p, xf.to_screen(*a), xf.to_screen(*b), text, c32(*color)),
            Overlay::GridLine { a, b, color } => {
                let c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                p.line_segment([xf.to_screen(*a), xf.to_screen(*b)], Stroke::new(1.0, c));
            }
            Overlay::Swatch { p: pt, color, selected } => {
                // A white disc under the colour shows its opacity; a dark rim keeps it readable on
                // any art, and the accent ring marks the selected stop.
                let c = xf.to_screen(*pt);
                p.circle_filled(c, 6.0, Color32::WHITE);
                p.circle_filled(c, 5.0, Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]));
                p.circle_stroke(c, 6.0, Stroke::new(1.0, Color32::from_gray(32)));
                if *selected {
                    p.circle_stroke(c, 8.0, Stroke::new(2.0, t.accent));
                }
            }
        }
    }
}

/// A Distance Guides measurement on screen ([`Overlay::Distance`]): the line from `a` to `b` in the
/// smart guide colour `color` with a 4 px tick across each end, and `text` in a pill of that colour
/// on its middle, in white or black, whichever reads on it.
fn distance_guide(p: &egui::Painter, a: Pos2, b: Pos2, text: &str, color: Color32) {
    let s = Stroke::new(1.0, color);
    p.line_segment([a, b], s);
    let along = b - a;
    let len = along.length();
    if len > 0.0 && len.is_finite() {
        let across = vec2(-along.y, along.x) / len * 4.0;
        for end in [a, b] {
            p.line_segment([end - across, end + across], s);
        }
    }
    let luma = 0.299 * f32::from(color.r()) + 0.587 * f32::from(color.g()) + 0.114 * f32::from(color.b());
    let ink = if luma > 150.0 { Color32::BLACK } else { Color32::WHITE };
    let galley = p.layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.0), ink);
    let r = egui::Rect::from_center_size(a + along / 2.0, galley.size() + vec2(8.0, 4.0));
    p.rect_filled(r, CornerRadius::same(3), color);
    p.galley(r.min + vec2(4.0, 2.0), galley, ink);
}
