//! Two interactive views through one camera, separated by a screen-space wipe.

use super::{Drag, Pos2, Tokens, Ui, VectorcraftApp, Xf, drag, vec2};
use crate::state::{SplitMode, SplitOrientation, SplitView};
use vectorcraft_engine::ViewInfo;

#[derive(Clone, Copy)]
struct DividerDrag {
    doc: u64,
    orientation: SplitOrientation,
    start: f32,
}

#[derive(Clone, Copy)]
struct HeldMode {
    doc: u64,
    outline: bool,
    pixel: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct ModeContext {
    doc: u64,
    outline: bool,
    pixel: bool,
    trim: bool,
    overprint: bool,
    comparison: SplitMode,
}

fn mode_id() -> egui::Id {
    egui::Id::unique("canvas-split-mode-context")
}

fn divider_id() -> egui::Id {
    egui::Id::unique("canvas-split-divider-drag")
}
fn held_id() -> egui::Id {
    egui::Id::unique("canvas-split-tool-mode")
}

pub(super) fn enabled(app: &VectorcraftApp) -> bool {
    app.ui.screen_mode < 3 && app.session.active().is_some() && app.view().is_some_and(|v| v.split.enabled)
}

pub(super) fn forget(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<DividerDrag>(divider_id());
        d.remove::<HeldMode>(held_id());
    });
}

/// Existing canvas cancellation (including a second finger or cancelled contact) also ends
/// a divider press; unlike Escape it retains the last fraction.
pub(super) fn cancel_divider(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<DividerDrag>(divider_id()));
}

pub(crate) fn dragging(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<DividerDrag>(divider_id()).is_some())
}

pub(super) fn suspend(app: &mut VectorcraftApp, ui: &Ui) {
    app.canvas.split.clear();
    ui.data_mut(|d| d.remove::<ModeContext>(mode_id()));
    if let Some(held) = ui.data(|d| d.get_temp::<HeldMode>(held_id()))
        && app.session.active().is_some_and(|st| st.uid == held.doc)
    {
        let mut view = app.view_info();
        view.outline = held.outline;
        view.pixel_preview = held.pixel;
        drag::drop_press(app, ui, view);
    }
    forget(ui.ctx());
}

pub(super) fn clips(rect: egui::Rect, split: SplitView) -> [egui::Rect; 2] {
    let mut first = rect;
    let mut second = rect;
    match split.orientation {
        SplitOrientation::Vertical => {
            let x = rect.left() + rect.width() * split.ratio();
            first.max.x = x;
            second.min.x = x;
        }
        SplitOrientation::Horizontal => {
            let y = rect.top() + rect.height() * split.ratio();
            first.max.y = y;
            second.min.y = y;
        }
    }
    [first, second]
}

fn hit(rect: egui::Rect, split: SplitView, p: Pos2) -> bool {
    if !rect.contains(p) {
        return false;
    }
    let edge = clips(rect, split)[0].max;
    match split.orientation {
        SplitOrientation::Vertical => (p.x - edge.x).abs() <= 3.0,
        SplitOrientation::Horizontal => (p.y - edge.y).abs() <= 3.0,
    }
}

pub(super) fn on_divider(app: &VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: egui::Rect) -> bool {
    if !enabled(app) || ui.data(|d| d.get_temp::<Drag>(drag::drag_id()).is_some()) {
        return false;
    }
    if dragging(ui.ctx()) {
        return true;
    }
    let (m, space, middle, at) = ui.input(|i| {
        (
            i.modifiers,
            i.key_down(egui::Key::Space),
            i.pointer.button_down(egui::PointerButton::Middle),
            i.pointer.hover_pos().or_else(|| i.pointer.interact_pos()),
        )
    });
    !middle
        && !drag::space_lends_view_tools(app, m, space)
        && at.is_some_and(|p| resp.hovered() && drag::on_canvas(ui, resp, p) && app.view().is_some_and(|v| hit(rect, v.split, p)))
}

/// Called after wheel/touch navigation, before any art/name press. An existing art press keeps
/// its owner even when it crosses the divider.
pub(super) fn divider_input(app: &mut VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: egui::Rect, modal: bool, space: bool) -> bool {
    let Some(st) = app.session.active() else {
        forget(ui.ctx());
        return false;
    };
    let uid = st.uid;
    let Some(settings) = app.view().map(|v| v.split).filter(|_| enabled(app)) else { return false };
    let held = ui.data(|d| d.get_temp::<DividerDrag>(divider_id()));
    if let Some(held) = held {
        if held.doc != uid || held.orientation != settings.orientation || modal {
            ui.data_mut(|d| d.remove::<DividerDrag>(divider_id()));
            return false;
        }
        let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let (down, at) = ui.input(|i| {
            let down = i.pointer.primary_down();
            let release = (!down)
                .then(|| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } => Some(*pos),
                        _ => None,
                    })
                })
                .flatten();
            (down, release.or_else(|| i.pointer.interact_pos()))
        });
        if escape {
            if let Some(v) = app.view_mut() {
                v.split.fraction = held.start;
            }
        } else if let Some(p) = at {
            let value = match settings.orientation {
                SplitOrientation::Vertical => (p.x - rect.left()) / rect.width().max(1.0),
                SplitOrientation::Horizontal => (p.y - rect.top()) / rect.height().max(1.0),
            };
            if let Some(v) = app.view_mut() {
                v.split.fraction = value.clamp(0.0, 1.0);
            }
        }
        if escape || !down {
            ui.data_mut(|d| d.remove::<DividerDrag>(divider_id()));
        }
        ui.ctx().request_repaint();
        return true;
    }
    if modal || space || crate::touch::finger_navigates_only() || !on_divider(app, ui, resp, rect) {
        return false;
    }
    if ui.input(|i| i.pointer.primary_pressed()) {
        ui.ctx().memory_mut(|mem| mem.stop_text_input());
        app.ui.flyout = None;
        // A same-frame tap has no drag left to hold.
        if ui.input(|i| i.pointer.primary_down()) {
            ui.data_mut(|d| d.insert_temp(divider_id(), DividerDrag { doc: uid, orientation: settings.orientation, start: settings.ratio() }));
        }
        app.session.tool_press_elsewhere();
    }
    app.session.tool_hover_ended(app.view_info());
    ui.data_mut(|d| d.insert_temp(egui::Id::unique("canvas-hovered"), false));
    true
}

pub(super) fn input_view(app: &VectorcraftApp, ui: &Ui, rect: egui::Rect, at: Option<Pos2>) -> ViewInfo {
    let mut view = app.view_info();
    if ui.data(|d| d.get_temp::<Drag>(drag::drag_id()).is_some_and(drag::tool_holds))
        && let Some(held) = ui.data(|d| d.get_temp::<HeldMode>(held_id()))
        && app.session.active().is_some_and(|st| st.uid == held.doc)
    {
        view.outline = held.outline;
        view.pixel_preview = held.pixel;
    } else if enabled(app)
        && let (Some(v), Some(p)) = (app.view(), at)
        && clips(rect, v.split)[1].contains(p)
    {
        view.outline = v.split.comparison == SplitMode::Outline;
        view.pixel_preview = v.split.comparison == SplitMode::PixelPreview;
    }
    view
}

pub(super) fn latch(app: &VectorcraftApp, ui: &Ui, view: ViewInfo) {
    if enabled(app)
        && let Some(st) = app.session.active()
    {
        ui.data_mut(|d| d.insert_temp(held_id(), HeldMode { doc: st.uid, outline: view.outline, pixel: view.pixel_preview }));
    }
}

pub(super) fn end_press(ui: &Ui) {
    if !ui.data(|d| d.get_temp::<Drag>(drag::drag_id()).is_some_and(drag::tool_holds)) {
        ui.data_mut(|d| d.remove::<HeldMode>(held_id()));
    }
}

pub(super) fn paper(app: &VectorcraftApp, ui: &Ui, painter: &egui::Painter, xf: &Xf, rect: egui::Rect, t: &Tokens) {
    let (Some(st), Some(v)) = (app.session.active(), app.view()) else { return };
    for (side, clip) in clips(rect, v.split).into_iter().enumerate() {
        let outline = if side == 0 { v.outline } else { v.split.comparison == SplitMode::Outline };
        super::paint_paper(app, st, ui, &painter.with_clip_rect(clip), xf, rect, (t, outline));
    }
}

pub(super) fn draw_art(app: &mut VectorcraftApp, ui: &Ui, painter: &egui::Painter, xf: &Xf, rect: egui::Rect, t: &Tokens) {
    let Some(v) = app.view().copied() else { return };
    let placed_generation = vectorcraft_render::placed_document::generation();
    let Some(st) = app.session.active() else { return };
    let mode = ModeContext {
        doc: st.uid,
        outline: v.outline,
        pixel: app.ui.view.pixel_preview,
        trim: app.ui.view.trim_view,
        overprint: vectorcraft_render::proof::overprint_preview_on(),
        comparison: v.split.comparison,
    };
    let mode_changed = ui.data_mut(|d| {
        let old = d.get_temp::<ModeContext>(mode_id());
        d.insert_temp(mode_id(), mode);
        old.is_some_and(|old| old != mode)
    });
    // Pixel Preview can have the same raster options below 100%; its logical mode still owns
    // a distinct pair context. Camera/pixel-region movement deliberately leaves this stamp alone.
    if mode_changed {
        app.canvas.split.clear();
    }
    let Some(first) = job(app, ui, xf, rect, t, None) else { return };
    let Some(mut second) = job(app, ui, xf, rect, t, Some(v.split.comparison)) else { return };
    // These asynchronous resources can advance between the two job preparations. Bind both
    // sides to one recorded generation/proof snapshot; the next frame observes any later one.
    second.key.plugins = first.key.plugins;
    second.key.placed = first.key.placed;
    second.opts.proof = first.opts.proof.clone();
    // Single-view results must still be drained while split owns the screen, so completed jobs
    // cannot keep an old document alive indefinitely or upload an unused texture later.
    for worker in [&mut app.canvas.worker, &mut app.canvas.fill_worker].into_iter().flatten() {
        let _ = worker.poll();
    }
    app.canvas.texture = None;
    app.canvas.key = None;
    app.canvas.shown = None;
    app.canvas.fill = None;
    app.canvas.split.update(ui.ctx(), [first, second]);
    if let Some(pair) = app.canvas.split.shown() {
        app.perf.render_ms = pair.ms;
        for (side, clip) in clips(rect, v.split).into_iter().enumerate() {
            if let (Some(texture), Some(key)) = (pair.textures.get(side), pair.keys.get(side)) {
                super::reproject_texture(&painter.with_clip_rect(clip), xf, rect, texture, key, v.rotation);
            }
        }
        // Retire only the document whose complete pair was drawn, never a merely queued view.
        if let Some(st) = app.session.active().filter(|st| pair.keys.iter().all(|key| key.doc == st.uid as usize)) {
            st.plugin_effects.first_view_done();
        }
    }
    // Bitmaps can start and finish while a synchronous pair is drawn. Their generation changes
    // before busy becomes false, so also wake for a completion since the pair was prepared.
    if vectorcraft_render::placed_document::busy() || vectorcraft_render::placed_document::generation() != placed_generation {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
    }
}

fn job(app: &mut VectorcraftApp, ui: &Ui, xf: &Xf, rect: egui::Rect, t: &Tokens, mode: Option<SplitMode>) -> Option<crate::render_worker::Job> {
    let st = app.session.active()?;
    let doc = st.doc.clone();
    let effects = st.plugin_effects.clone();
    let mask_view = st.shown_mask();
    let drawn = app.view()?.on_screen(app.canvas_ui_zoom);
    let device_ppp = ui.ctx().pixels_per_point();
    let max_side = ui.ctx().input(|i| i.max_texture_side);
    let ppp = super::frame_ppp(rect.size(), device_ppp, max_side);
    let (w, h) = (
        ((rect.width() * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX)),
        ((rect.height() * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX)),
    );
    let pixel_mode = mode.map_or(app.ui.view.pixel_preview, |m| m == SplitMode::PixelPreview);
    let pixel = (pixel_mode && xf.zoom * f64::from(device_ppp) > 1.0).then(|| super::pixel_region(xf, max_side)).flatten();
    let smooth = pixel.is_none() || app.session.prefs.anti_aliased_bitmaps;
    let plugins = super::plugin_key(&mut app.canvas.plugin_keys, &app.session, st);
    let placed = super::placed_key(&mut app.canvas.placed_keys, &app.session, st);
    let mut key = super::cache_key(app, ui, st, t, &drawn, (w, h, ppp), (pixel, smooth, plugins, placed));
    if let Some(mode) = mode {
        key.outline = mode == SplitMode::Outline;
        key.trim = mode == SplitMode::TrimView;
        key.backdrop = super::backdrop(app, st, key.outline);
    }
    let mut opts = super::frame_options(app, &key, mask_view, smooth);
    if let Some(mode) = mode {
        opts.outline = mode == SplitMode::Outline;
        opts.trim = mode == SplitMode::TrimView;
        opts.overprint_preview = mode == SplitMode::OverprintPreview;
    }
    let (w, h, view) = super::frame_view(pixel, (w, h), &drawn, ppp);
    Some(crate::render_worker::Job {
        key,
        doc,
        w,
        h,
        view,
        opts,
        scope: vectorcraft_engine::file_access::ScopeHandle::capture(),
        effects,
        may_copy: false,
        may_keep: false,
        display: app.display_profile.transform(app.session.prefs.monitor_profile_display),
    })
}

pub(super) fn mode_overlays(app: &VectorcraftApp, ui: &Ui, painter: &egui::Painter, xf: &Xf, rect: egui::Rect, t: &Tokens) {
    let Some(v) = app.view() else { return };
    for (side, clip) in clips(rect, v.split).into_iter().enumerate() {
        let painter = painter.with_clip_rect(clip);
        let pixel = if side == 0 { app.ui.view.pixel_preview } else { v.split.comparison == SplitMode::PixelPreview };
        if pixel && v.zoom >= super::PIXEL_GRID_ZOOM && app.session.prefs.show_pixel_grid {
            super::grid(&painter, xf, 1.0, 1, super::LineLook { color: super::PIXEL_GRID, dots: false });
        }
        let outline = if side == 0 { v.outline } else { v.split.comparison == SplitMode::Outline };
        super::artboard_outlines_mode(app, ui, &painter, xf, t, outline);
    }
}

pub(super) fn divider(app: &VectorcraftApp, painter: &egui::Painter, rect: egui::Rect, t: &Tokens, visible: bool) {
    if !visible {
        return;
    }
    let Some(v) = app.view() else { return };
    let edge = clips(rect, v.split)[0].max;
    let (a, b, grip) = match v.split.orientation {
        SplitOrientation::Vertical => (
            egui::pos2(edge.x, rect.top()),
            egui::pos2(edge.x, rect.bottom()),
            egui::Rect::from_center_size(egui::pos2(edge.x, rect.center().y), vec2(5.0, 24.0)),
        ),
        SplitOrientation::Horizontal => (
            egui::pos2(rect.left(), edge.y),
            egui::pos2(rect.right(), edge.y),
            egui::Rect::from_center_size(egui::pos2(rect.center().x, edge.y), vec2(24.0, 5.0)),
        ),
    };
    painter.line_segment([a, b], egui::Stroke::new(3.0, t.panel));
    painter.line_segment([a, b], egui::Stroke::new(1.0, t.border));
    painter.rect_filled(grip, 2.0, t.text_dim);
}

pub(super) fn cursor(app: &VectorcraftApp, ui: &Ui) -> bool {
    let Some(rect) = app.canvas_rect else { return false };
    let Some(v) = app.view().filter(|_| enabled(app)) else { return false };
    let (at, m, space, middle) =
        ui.input(|i| (i.pointer.hover_pos(), i.modifiers, i.key_down(egui::Key::Space), i.pointer.button_down(egui::PointerButton::Middle)));
    let tool_holds = ui.data(|d| d.get_temp::<Drag>(drag::drag_id()).is_some());
    if dragging(ui.ctx()) || (!tool_holds && !middle && !drag::space_lends_view_tools(app, m, space) && at.is_some_and(|p| hit(rect, v.split, p))) {
        ui.ctx().set_cursor_icon(match v.split.orientation {
            SplitOrientation::Vertical => egui::CursorIcon::ResizeHorizontal,
            SplitOrientation::Horizontal => egui::CursorIcon::ResizeVertical,
        });
        return true;
    }
    false
}
