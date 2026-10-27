use super::grid::{clip_segment, pref_color};
use super::outline::{OVERLAY_MAX_ANCHORS, c32, stroke_path};
use super::*;

/// Type → Show Hidden Characters: spaces as dots, ¶ at paragraph ends, # at the end of a story.
/// The marks of each text object are kept from one frame to the next ([`HiddenChars`]).
pub(super) fn hidden_chars_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let clip = p.clip_rect();
    let color = Color32::from_rgb(0x4f, 0x9d, 0xff);
    let font = egui::FontId::proportional(((10.0 * xf.zoom) as f32).clamp(7.0, 18.0));
    let kept = &mut app.canvas.hidden_chars;
    // `last` holds the marks of the last frame. The marks of the objects this frame doesn't draw
    // are dropped at the end.
    let mut last = std::mem::take(&mut kept.marks);
    // The inputs are read before any layout: when fonts load during this frame, the next frame
    // lays all type out again.
    let inputs = (vectorcraft_text::FontDb::global().generation(), vectorcraft_text::hyphenation_exceptions());
    if kept.inputs != inputs {
        last.clear();
        kept.inputs = inputs;
    }
    // `walk` visits each node's `Arc` in the order `Document::walk` visits the nodes (depth first,
    // in paint order).
    fn walk(nodes: &[std::sync::Arc<Node>], f: &mut impl FnMut(&std::sync::Arc<Node>)) {
        for n in nodes {
            f(n);
            if let Some(children) = n.children() {
                walk(children, f);
            }
        }
    }
    walk(&st.doc.layers, &mut |n| {
        let NodeKind::Text(tx) = &n.kind else { return };
        // The box round the object's corners on screen, turned with the view.
        if !n.visible || n.geometric_bounds().is_none_or(|b| !egui::Rect::from_points(&xf.quad(b)).intersects(clip)) {
            return;
        }
        let marks = match last.remove(&n.id) {
            Some((was, marks)) if std::sync::Arc::ptr_eq(&was, n) => marks,
            _ => std::sync::Arc::new(HiddenMarks::of(tx)),
        };
        for &at in &marks.spaces {
            p.circle_filled(xf.to_screen(at), 1.2, color);
        }
        for &(at, glyph) in &marks.ends {
            p.text(xf.to_screen(at), egui::Align2::LEFT_BOTTOM, glyph, font.clone(), color);
        }
        kept.marks.insert(n.id, (n.clone(), marks));
    });
}

/// The Show Hidden Characters marks of the text objects drawn in the last frame
/// ([`crate::CanvasCache::hidden_chars`]). Each is kept with the object it was laid out from: an
/// edited object is a new `Arc` and is laid out again, and all type is laid out again when the
/// fonts or the hyphenation exceptions change.
#[derive(Default)]
pub struct HiddenChars {
    /// The fonts' generation ([`vectorcraft_text::FontDb::generation`]) and the hyphenation
    /// exceptions the marks were laid out with.
    pub(super) inputs: (u64, String),
    /// Each text object drawn, by id: the node it was laid out from and its marks.
    pub(super) marks: std::collections::HashMap<vectorcraft_doc::NodeId, (std::sync::Arc<Node>, std::sync::Arc<HiddenMarks>)>,
}

/// One text object's marks in document space: a dot over each space, and ¶ or # after the lines
/// that end a paragraph or the story. Type on a path has none.
#[derive(Default)]
pub(super) struct HiddenMarks {
    pub(super) spaces: Vec<Point>,
    ends: Vec<(Point, &'static str)>,
}

impl HiddenMarks {
    /// Lay out `tx` and place its marks.
    pub(super) fn of(tx: &vectorcraft_doc::TextObject) -> Self {
        let mut marks = Self::default();
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx);
        if lay.on_path {
            return marks;
        }
        let text = tx.plain_text();
        let to = |q: Point| tx.xf * q;
        // A list marker's glyphs (length 0) stand for no character.
        for g in lay.glyphs.iter().filter(|g| g.len > 0) {
            if text.get(g.byte..).is_some_and(|s| s.starts_with(' ')) {
                marks.spaces.push(to(Point::new(g.origin.x + g.advance / 2.0, g.origin.y - 3.0)));
            }
        }
        for (i, line) in lay.lines.iter().enumerate() {
            let last = i + 1 == lay.lines.len();
            let breaks = text.get(line.end..).is_some_and(|s| s.starts_with('\n'));
            let (glyph, at) = if last && line.end >= text.len() {
                ("#", true)
            } else if breaks {
                ("¶", true)
            } else {
                ("", false)
            };
            if at {
                marks.ends.push((to(Point::new(line.x1 + 1.0, line.baseline)), glyph));
            }
        }
        marks
    }
}

/// View › Show Text Threads: for each thread with a selected frame, a line in its first frame's
/// layer colour from each frame's out port to the next frame's in port, both showing the arrow of a
/// threaded port, where the tools draw the ports of selected type ([`vectorcraft_tools::text`]).
/// Only frames with ports ([`vectorcraft_tools::text::ported`]) are joined: a link to or from a
/// frame in perspective, which has none, gets no line. With more than
/// [`vectorcraft_tools::text::PORTS_MAX`] such frames selected the tools draw no ports, and only
/// the lines are drawn here. `zoom`: screen pixels to the document unit.
pub(super) fn thread_overlays(app: &VectorcraftApp, zoom: f64) -> Vec<Overlay> {
    use vectorcraft_tools::text::{Port, PortState, port_centre, ported, ports_shown, push_port};
    let Some(st) = app.session.active() else { return vec![] };
    let px = 1.0 / zoom.max(1e-9);
    let boxes = ports_shown(&st.doc, &st.selection.objects);
    let mut o = vec![];
    for thread in &st.doc.text_threads {
        if !thread.iter().any(|id| st.selection.objects.contains(id)) {
            continue;
        }
        let color = st.doc.layer_color(thread[0]);
        for w in thread.windows(2) {
            let [a, b] = [w[0], w[1]];
            let (Some(from), Some(to)) = (ported(&st.doc, a), ported(&st.doc, b)) else { continue };
            let (Some(out), Some(input)) = (port_centre(from, Port::Out, px), port_centre(to, Port::In, px)) else { continue };
            o.push(Overlay::Line { a: out, b: input, color, dashed: false });
            if boxes {
                push_port(&mut o, out, PortState::Threaded, st.doc.layer_color(a), px);
                push_port(&mut o, input, PortState::Threaded, st.doc.layer_color(b), px);
            }
        }
    }
    o
}

/// View → Show Live Paint Gaps: the gaps of the selected Live Paint groups in each group's Gap
/// Preview Color ([`crate::dialogs::live_paint_gaps::selected_gaps`]). Cached per revision.
pub(super) fn live_paint_gap_overlays(app: &mut VectorcraftApp) -> Vec<Overlay> {
    let Some(st) = app.session.active() else { return vec![] };
    let key = (st.uid, st.revision);
    let gaps = match &app.canvas.live_paint_gaps {
        Some((k, gaps)) if *k == key => gaps.clone(),
        _ => {
            let gaps = std::sync::Arc::new(crate::dialogs::live_paint_gaps::selected_gaps(app));
            app.canvas.live_paint_gaps = Some((key, gaps.clone()));
            gaps
        }
    };
    gaps.to_vec()
}

/// Simplify's Show Original Path: the paths its preview ran on, as they were before it, in a band
/// 3 screen points wide of their layer's color at 40%, under the selection's outlines. Nothing is drawn
/// when they have more than [`OVERLAY_MAX_ANCHORS`] anchor points.
pub(super) fn original_path_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let paths = crate::dialogs::path_ops::original_paths(app);
    let Some(st) = app.session.active().filter(|_| paths.iter().map(|(_, path)| path.anchor_count()).sum::<usize>() <= OVERLAY_MAX_ANCHORS) else {
        return;
    };
    for (id, path) in paths {
        stroke_path(p, &path.to_bezpath(), xf, Stroke::new(3.0, c32(st.doc.layer_color(id)).gamma_multiply(0.4)));
    }
}

/// The slices (unless View → Hide Slices): user and object slices outlined in the Slices
/// preferences' line colour, auto slices dashed and dimmer, each numbered at its top left (Show
/// Slice Numbers); selected slices have a bolder outline. The layout is cached per revision.
pub(super) fn slice_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let key = (st.uid, st.revision);
    let slices = match &app.canvas.slices {
        Some((k, s)) if *k == key => s.clone(),
        _ => {
            let s = std::sync::Arc::new(st.doc.slice_layout());
            app.canvas.slices = Some((key, s.clone()));
            s
        }
    };
    if slices.is_empty() {
        return;
    }
    let prefs = &app.session.prefs;
    let line = pref_color(&prefs.slice_line_color, Color32::from_rgb(0xff, 0x3f, 0x3f));
    let dim = line.gamma_multiply(0.55);
    let selected = &st.selection.slices;
    let objects = &st.selection.objects;
    for a in slices.iter() {
        let auto = a.source == vectorcraft_doc::SliceSource::Auto;
        let on = a.id.is_some_and(|id| selected.contains(&id) || objects.contains(&id));
        let q = xf.quad(a.rect);
        let ring: Vec<Pos2> = q.iter().chain(q.first()).copied().collect();
        if auto {
            p.extend(Shape::dashed_line(&ring, Stroke::new(1.0, dim), 3.0, 3.0));
        } else {
            p.add(Shape::line(ring, Stroke::new(if on { 2.0 } else { 1.0 }, line)));
        }
        if prefs.show_slice_numbers
            && let Some(tl) = q.first()
        {
            let font = egui::FontId::proportional(9.0);
            let galley = p.layout_no_wrap(format!("{:02}", a.number), font, Color32::WHITE);
            let badge = egui::Rect::from_min_size(*tl + vec2(1.0, 1.0), galley.size() + vec2(6.0, 2.0));
            p.rect_filled(badge, CornerRadius::ZERO, if auto { dim } else { line });
            p.galley(badge.min + vec2(3.0, 1.0), galley, Color32::WHITE);
        }
    }
}

/// The print tiling (View → Show Print Tiling, or the Print Tiling tool): each page of the
/// document's print settings as `print.preview` lays it out, its paper edge and its imageable
/// area dashed, numbered at its top left; tiles outside the tile range dimmer. Cached per revision.
pub(super) fn print_tiling_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let key = (st.uid, st.revision);
    let pages = match &app.canvas.print_tiling {
        Some((k, pages)) if *k == key => pages.clone(),
        _ => {
            // Settings that can't print have no pages to show.
            let pages = std::sync::Arc::new(vectorcraft_engine::cmd::printtiling::pages(&st.doc).unwrap_or_default());
            app.canvas.print_tiling = Some((key, pages.clone()));
            pages
        }
    };
    let font = egui::FontId::proportional(10.0);
    let ring = |r: [f64; 4]| {
        let q = xf.quad(Rect::new(r[0], r[1], r[2], r[3]));
        q.iter().chain(q.first()).copied().collect::<Vec<Pos2>>()
    };
    for page in pages.iter() {
        let color = if page.printed { PRINT_TILING } else { PRINT_TILING.gamma_multiply(0.45) };
        p.add(Shape::line(ring(page.page), Stroke::new(0.75, color)));
        let inner = ring(page.imageable);
        p.extend(Shape::dashed_line(&inner, Stroke::new(1.0, color), 4.0, 3.0));
        if let Some(corner) = inner.iter().copied().reduce(|a, b| pos2(a.x.min(b.x), a.y.min(b.y))) {
            p.text(corner + vec2(3.0, 2.0), egui::Align2::LEFT_TOP, page.number.to_string(), font.clone(), color);
        }
    }
}

/// The print tiling's lines: a neutral grey that reads on the paper and on the pasteboard.
const PRINT_TILING: Color32 = Color32::from_gray(96);

/// A Shaper Group's mode widget (Shaper and Selection tools), drawn with selection edges shown or
/// hidden: an arrow pointing right in Face Selection mode and left in Construction mode.
pub(super) fn shaper_widget_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    if matches!(app.session.tool_id(), "shaper" | "selection")
        && let Some(w) = vectorcraft_tools::shaperwidget::ShaperWidget::of(&st.doc, &st.selection, st.isolation, xf.zoom)
    {
        let (c, r) = (xf.to_screen(w.at), vectorcraft_tools::shaperwidget::RADIUS_PX);
        let color = c32(st.doc.layer_color(w.group));
        p.circle_filled(c, r, Color32::WHITE);
        p.circle_stroke(c, r, Stroke::new(1.0, color));
        let d = if w.construction { -1.0 } else { 1.0 };
        let arrow = vec![c + vec2(3.0 * d, 0.0), c + vec2(-2.0 * d, -3.0), c + vec2(-2.0 * d, 3.0)];
        p.add(Shape::convex_polygon(arrow, color, Stroke::NONE));
    }
}

/// High-contrast live controls, using the same geometry as the Selection tool's hit testing.
pub(super) fn mirror_axis_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    if app.session.tool_id() != "selection" || !app.ui.view.bounding_box {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(axis) = vectorcraft_tools::mirror::MirrorAxis::of(&st.doc, &st.selection, xf.zoom) else { return };
    let t = Tokens::get(p.ctx());
    let ends = axis.ends.map(|q| xf.to_screen(q));
    let center = xf.to_screen(axis.center);
    if !center.is_finite() || !ends.iter().all(|p| p.is_finite()) {
        return;
    }
    if let Some((a, b)) = clip_segment(ends[0], ends[1], xf.rect) {
        p.line_segment([a, b], Stroke::new(5.0, t.measure_bg));
        p.line_segment([a, b], Stroke::new(2.0, t.selection));
    }
    for end in ends {
        p.circle_filled(end, vectorcraft_tools::mirror::HANDLE_RADIUS, t.measure_bg);
        p.circle_stroke(end, vectorcraft_tools::mirror::HANDLE_RADIUS, Stroke::new(2.0, t.selection));
    }
    let handle = egui::Rect::from_center_size(center, vec2(12.0, 12.0));
    p.rect_filled(handle, 2.0, t.selection);
    p.rect_stroke(handle, 2.0, Stroke::new(2.0, t.measure_bg), StrokeKind::Outside);
}

/// A selected grid repeat's row, column and corner handles and its spacing sliders (#2691), at
/// the places the Selection tool's hit testing finds them.
pub(super) fn grid_repeat_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    use vectorcraft_tools::repeat_grid::{GridHandles, GridPart, HANDLE_RADIUS, SLIDER_HALF};
    if app.session.tool_id() != "selection" || !app.ui.view.bounding_box {
        return;
    }
    let frame = app.selection_box();
    let Some(st) = app.session.active() else { return };
    let Some(grid) = GridHandles::of(&st.doc, &st.selection, frame, xf.zoom) else { return };
    let t = Tokens::get(p.ctx());
    let half = f64::from(SLIDER_HALF) / xf.zoom;
    for (part, at) in grid.parts() {
        let c = xf.to_screen(at);
        if !c.is_finite() {
            continue;
        }
        if part.is_slider() {
            // A bar across the gap's middle, square to the spacing it sets.
            let along = if part == GridPart::HSpacing { vectorcraft_geom::Vec2::new(0.0, half) } else { vectorcraft_geom::Vec2::new(half, 0.0) };
            let ends = [xf.to_screen(at - along), xf.to_screen(at + along)];
            p.line_segment(ends, Stroke::new(6.0, t.measure_bg));
            p.line_segment(ends, Stroke::new(2.5, t.selection));
        } else {
            p.circle_filled(c, HANDLE_RADIUS, t.measure_bg);
            p.circle_stroke(c, HANDLE_RADIUS, Stroke::new(2.0, t.selection));
        }
    }
}

/// The radius of the Rotate View dial, in screen points.
pub(super) const DIAL_RADIUS: f32 = 40.0;

/// The dial a Rotate View drag shows, centered on `c`: a tick every 15° (longer every 90°), a
/// needle pointing where the document's top points in a view turned by `rotation` degrees, and the
/// angle under the center, rounded to whole degrees as in the status bar.
pub(super) fn rotate_view_dial(p: &egui::Painter, c: Pos2, rotation: f64, t: &Tokens) {
    // `dir(deg)` is the screen direction `deg` degrees clockwise from straight up.
    let dir = |deg: f32| vec2(deg.to_radians().sin(), -deg.to_radians().cos());
    p.circle(c, DIAL_RADIUS, t.panel.gamma_multiply(0.85), Stroke::new(1.0, t.border));
    for i in 0..24u8 {
        let len = if i % 6 == 0 { 8.0 } else { 4.0 };
        let d = dir(f32::from(i) * 15.0);
        p.line_segment([c + d * (DIAL_RADIUS - len), c + d * DIAL_RADIUS], Stroke::new(1.0, t.text_dim));
    }
    p.line_segment([c, c + dir(rotation as f32) * DIAL_RADIUS], Stroke::new(2.0, t.accent));
    p.circle_filled(c, 2.5, t.accent);
    p.text(c + vec2(0.0, 8.0), egui::Align2::CENTER_TOP, format!("{rotation:.0}°"), egui::FontId::proportional(11.0), t.text);
}
