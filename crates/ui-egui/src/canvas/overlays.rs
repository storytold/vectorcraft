use super::artboards::names_an_artboard;
use super::input::marquee_through;
use super::outline::{KEY_OUTLINE, c32, node_outline, selection_hit_at, stroke_path, to_screen_path, too_many_anchors, walk_drawn};
use super::{Color32, CornerRadius, NodeKind, Overlay, Point, Pos2, Shape, Stroke, StrokeKind, Tokens, VectorcraftApp, Xf, vec2};

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
    grow: f32,
    /// `solid` (the default), `hollow` or `large`.
    style: &'a str,
}

impl<'a> HandleLook<'a> {
    pub(super) fn of(prefs: &'a vectorcraft_engine::Prefs) -> Self {
        Self { grow: prefs.anchor_size.clamp(1, 7) as f32 - 3.0, style: &prefs.handle_style }
    }

    /// A direction handle's end at `c`: a solid dot, a hollow one or a larger solid one.
    fn draw(self, p: &egui::Painter, c: Pos2, color: Color32) {
        let r = 2.75 + self.grow / 2.0;
        let hollow = self.style == "hollow";
        p.circle_filled(c, if self.style == "large" { r + 1.5 } else { r }, if hollow { Color32::WHITE } else { color });
        if hollow {
            p.circle_stroke(c, r, Stroke::new(1.0, color));
        }
    }
}

fn anchor_square(p: &egui::Painter, c: Pos2, color: Color32, filled: bool, size: f32) {
    let r = egui::Rect::from_center_size(c, vec2(size, size));
    if filled {
        p.rect_filled(r, 0.0, color);
    } else {
        p.rect_filled(r, 0.0, Color32::WHITE);
        p.rect_stroke(r, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
    }
}

fn curve_point(p: &egui::Painter, c: Pos2, color: Color32, filled: bool, size: f32) {
    p.circle(c, size / 2.0, if filled { color } else { Color32::WHITE }, Stroke::new(1.0, color));
}

pub(super) fn selection_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf, curve_points_from_tool: bool) {
    // The Selection tool's bounding box (rotated with the objects after a rotation). It hides while
    // a drag moves or resizes the selection, so only the art is seen going with the pointer (#712).
    let show_box = app.session.tool_id() == "selection"
        && !app.session.tool_transforming()
        && app.ui.view.bounding_box
        && app.session.active().is_some_and(|st| !st.selection.is_empty() && st.selection.anchors.is_empty());
    let bbox = if show_box { app.selection_box() } else { None };
    let st = app.session.active();
    let big = st.is_some_and(|st| too_many_anchors(st.selection.objects.iter().filter_map(|id| st.doc.node(*id))));
    let big_bounds = if big { app.selection_bounds() } else { None };
    let app = &*app;
    let Some(st) = app.session.active() else { return };
    let symbols = st.doc.symbol_boxes();
    let tool = app.session.tool_id();
    let direct = vectorcraft_tools::catalog::edits_anchors(tool);
    let curvature = tool == "curvature";
    // Selection & Anchor Display › Size (1–7, 3 the default): anchors, handles and the bounding
    // box's handles a point bigger or smaller per step.
    let look = HandleLook::of(&app.session.prefs);
    let grow = look.grow;
    let anchor = |direct: bool| grow + if direct { 5.0 } else { 4.0 };
    // The selected anchors' handles (drawn once they are counted: Show handles when multiple
    // anchors are selected off shows them for a single one only).
    let (mut handles, mut with_handles) = (vec![], 0);
    // Too many paths to outline: the selection's bounds stand for them.
    if let Some((b, id)) = big_bounds.zip(st.selection.objects.first()) {
        stroke_path(p, &vectorcraft_geom::shapes::rectangle(b).to_bezpath(), xf, Stroke::new(1.0, c32(st.doc.layer_color(*id))));
    }
    // The key object (Align to Key Object): its outline drawn thicker, or its bounds when it has
    // too many paths to outline.
    if let Some((k, n)) = st.selection.key.and_then(|k| Some((k, st.doc.node(k)?))) {
        let outline = if too_many_anchors([n]) {
            n.geometric_bounds_with(&symbols).map(|b| vectorcraft_geom::shapes::rectangle(b).to_bezpath())
        } else {
            Some(node_outline(n, &symbols))
        };
        if let Some(bp) = outline {
            stroke_path(p, &bp, xf, Stroke::new(KEY_OUTLINE, c32(st.doc.layer_color(k))));
        }
    }
    for id in st.selection.objects.iter().filter(|_| !big) {
        let Some(n) = st.doc.node(*id) else { continue };
        let color = c32(st.doc.layer_color(*id));
        let partial = st.selection.partial(*id);
        // Path outlines (and area type / placed document / image / mesh / symbol outlines;
        // point type baselines and origin are drawn below).
        if !matches!(n.kind, NodeKind::Text(ref tx) if matches!(tx.kind, vectorcraft_doc::TextKind::Point)) {
            stroke_path(p, &node_outline(n, &symbols), xf, Stroke::new(1.0, color));
        }
        // Anchors (and handles for selected anchors in direct mode); a gradient mesh's or a mesh
        // envelope's points.
        walk_drawn(n, &mut |c| {
            let envelope = vectorcraft_doc::live::envelope_overlay(c).and_then(|(_, grid)| grid);
            let grid = match &c.kind {
                NodeKind::Mesh(m) => Some(m),
                _ => envelope.as_ref(),
            };
            if let Some(grid) = grid {
                for q in &grid.points {
                    anchor_square(p, xf.to_screen(q.p), color, false, anchor(direct));
                }
                return;
            }
            let NodeKind::Path { path, .. } = &c.kind else { return };
            // The Curvature tool owns its points while drawing; existing paths use this overlay.
            if curvature && curve_points_from_tool {
                return;
            }
            let selected_segments = st.selection.segments.get(&c.id);
            if let Some(segments) = selected_segments {
                for &(si, seg) in segments {
                    let Some(sp) = path.subpaths.get(si).filter(|sp| seg < sp.segment_count()) else { continue };
                    let curve = sp.segment(seg);
                    let mut outline = vectorcraft_geom::BezPath::new();
                    outline.move_to(curve.p0);
                    if sp.segment_is_line(seg) {
                        outline.line_to(curve.p3);
                    } else {
                        outline.curve_to(curve.p1, curve.p2, curve.p3);
                    }
                    stroke_path(p, &outline, xf, Stroke::new(2.0, color));
                }
            }
            for (si, ai, a) in path.anchors() {
                // An object selected whole has all its anchors selected, and so do the paths inside a
                // group, compound path or clip group selected whole. Direction handles show only on a
                // path that is selected itself: those are the handles that `handle_anchors` in
                // `vectorcraft-tools` lets the tools drag.
                let (sel, handles_shown) = match partial {
                    Some(set) => {
                        let sel = set.contains(&(si, ai));
                        (sel, sel)
                    }
                    None => (true, direct && c.id == *id),
                };
                let sp = xf.to_screen(a.p);
                if !curvature && handles_shown {
                    with_handles += 1;
                    let sides = path.subpaths.get(si).map(|subpath| st.selection.handle_sides(c.id, si, ai, subpath)).unwrap_or((true, true));
                    handles.extend(
                        [(a.h_in, sides.0), (a.h_out, sides.1)]
                            .into_iter()
                            .filter(|(h, show)| *show && h.distance(a.p) > 1e-6)
                            .map(|(h, _)| (sp, xf.to_screen(h), color)),
                    );
                }
                let anchor_selected = sel && selected_segments.is_none();
                let ink = if anchor_selected && partial.is_some() { selected_anchor(color) } else { color };
                if curvature {
                    curve_point(p, sp, ink, anchor_selected && partial.is_some(), grow + 6.0);
                } else {
                    anchor_square(p, sp, ink, anchor_selected, anchor(partial.is_some() || direct));
                }
            }
        });
        // Centre point (Attributes panel → Show Center).
        if n.shows_center()
            && let Some(b) = n.geometric_bounds_with(&symbols)
        {
            anchor_square(p, xf.to_screen(b.center()), color, true, anchor(false));
        }
        // Point type: each line's baseline, where a click selects it (`hit`), whatever its
        // alignment and rotation, and the anchor (area type shows its frame instead, which may
        // not be a rectangle, and type on a path its path).
        if let NodeKind::Text(tx) = &n.kind
            && matches!(tx.kind, vectorcraft_doc::TextKind::Point)
        {
            for (a, b) in tx.baselines() {
                p.line_segment([xf.to_screen(tx.xf * a), xf.to_screen(tx.xf * b)], Stroke::new(1.0, color));
            }
            p.circle_filled(xf.to_screen(tx.xf * Point::ZERO), 2.5, color);
        }
    }
    if app.session.prefs.show_handles_multiple_anchors || with_handles <= 1 {
        for (sp, hp, color) in handles {
            p.line_segment([sp, hp], Stroke::new(1.0, color));
            look.draw(p, hp, color);
        }
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
    // Live Corners widgets (Selection on a live rectangle or polygon, Direct Selection on any path).
    if matches!(tool, "selection" | "directSelection")
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
    if tool == "selection" && app.ui.view.bounding_box {
        for w in vectorcraft_tools::pie::PieWidget::of(&st.doc, &st.selection, xf.zoom) {
            let (on, c, r, color) = (xf.to_screen(w.on), xf.to_screen(w.at), vectorcraft_tools::pie::RADIUS_PX, c32(st.doc.layer_color(w.id)));
            p.line_segment([on, c], Stroke::new(1.0, color));
            p.circle_filled(c, r, Color32::WHITE);
            p.circle_stroke(c, r, Stroke::new(1.0, color));
        }
    }
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
                p.rect_filled(r, CornerRadius::same(3), t.measure_bg);
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
