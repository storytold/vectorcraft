//! Two interactive views through one camera, separated by a screen-space wipe.

use super::{Drag, Pos2, Tokens, Ui, VectorcraftApp, Xf, drag, vec2};
use crate::state::{SplitMode, SplitOrientation, SplitView};
use vectorcraft_engine::ViewInfo;

#[derive(Clone, Copy)]
struct DividerDrag {
    doc: u64,
    view: u32,
    orientation: SplitOrientation,
    start: f32,
    motion: DividerMotion,
}

#[derive(Clone, Copy)]
struct DividerMotion {
    axis: f32,
    moved: bool,
}

impl DividerMotion {
    fn new(orientation: SplitOrientation, p: Pos2) -> Self {
        Self { axis: divider_axis(orientation, p), moved: false }
    }
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

/// The last primary press in this frame, with its event index.
pub(crate) fn primary_press(ui: &Ui) -> Option<(usize, Pos2)> {
    ui.input(|i| {
        i.events.iter().enumerate().rev().find_map(|(index, e)| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some((index, *pos)),
            _ => None,
        })
    })
}

/// The first primary release, optionally restricted to events after a particular press.
pub(crate) fn primary_release(ui: &Ui, after: Option<usize>) -> Option<(usize, Pos2)> {
    ui.input(|i| {
        i.events.iter().enumerate().find_map(|(index, e)| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. }
                if after.is_none_or(|press| index > press) =>
            {
                Some((index, *pos))
            }
            _ => None,
        })
    })
}

/// The first complete primary pair in this frame's prefix, after an already finished owner.
pub(super) fn completed_primary_press(ui: &Ui, after: Option<usize>, before: usize) -> Option<(usize, Pos2, usize, Pos2)> {
    let (down, pressed) = ui.input(|i| {
        i.events.iter().enumerate().find_map(|(index, event)| match event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. }
                if index < before && after.is_none_or(|up| index > up) =>
            {
                Some((index, *pos))
            }
            _ => None,
        })
    })?;
    let (up, released) = primary_release(ui, Some(down))?;
    (up < before).then_some((down, pressed, up, released))
}

/// Whether the new primary press belongs to the divider, using that event's position.
pub(super) fn press_on_divider(app: &VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: egui::Rect) -> bool {
    enabled(app)
        && !ui.data(|d| d.get_temp::<Drag>(drag::drag_id()).is_some())
        && !ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle))
        && primary_press(ui).is_some_and(|(down, p)| !drag::primary_press_state(app, ui, down).1 && divider_press_at(app, ui, resp, rect, p))
}

pub(super) fn divider_press_at(app: &VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: egui::Rect, p: Pos2) -> bool {
    drag::on_canvas(ui, resp, p) && app.view().is_some_and(|v| hit(rect, v.split, p))
}

/// The first completed divider press before this frame's latest Down. This read-only query
/// also keeps fresh mapped handles from taking a later Down before the divider can finish.
pub(super) fn completed_press_on_divider(app: &VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: egui::Rect) -> bool {
    completed_divider_press(app, ui, resp, rect, None, None).is_some()
}

fn completed_divider_press(
    app: &VectorcraftApp,
    ui: &Ui,
    resp: &egui::Response,
    rect: egui::Rect,
    after: Option<usize>,
    before: Option<usize>,
) -> Option<(usize, Pos2, usize, Pos2)> {
    if !enabled(app) || ui.data(|d| d.get_temp::<Drag>(drag::drag_id()).is_some()) || ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle))
    {
        return None;
    }
    let latest = before.or_else(|| primary_press(ui).map(|(down, _)| down))?;
    let presses = ui.input(|i| {
        i.events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| match event {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. }
                    if index < latest && after.is_none_or(|up| index > up) =>
                {
                    Some((index, *pos))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    });
    presses.into_iter().find_map(|(down, p)| {
        let (up, released) = primary_release(ui, Some(down))?;
        (up < latest && !drag::primary_press_state(app, ui, down).1 && divider_press_at(app, ui, resp, rect, p)).then_some((down, p, up, released))
    })
}

/// Finish only the old divider's release before navigation can handle a later press.
/// Escape remains owned by divider_input, which restores the original fraction.
pub(super) fn finish_held_divider_before_press(app: &mut VectorcraftApp, ui: &Ui, rect: egui::Rect, modal: bool) -> Option<usize> {
    if modal || !enabled(app) || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        return None;
    }
    let mut held = ui.data(|d| d.get_temp::<DividerDrag>(divider_id()))?;
    let uid = app.session.active()?.uid;
    let (_, view) = crate::doc_windows::current(app)?;
    let settings = app.view()?.split;
    if held.doc != uid || held.view != view || held.orientation != settings.orientation {
        return None;
    }
    let release = primary_release(ui, None)?;
    let (down, _) = primary_press(ui)?;
    if down <= release.0 {
        return None;
    }
    move_divider(app, ui, rect, held.orientation, &mut held.motion, (None, Some(release)));
    ui.data_mut(|d| d.remove::<DividerDrag>(divider_id()));
    ui.ctx().request_repaint();
    Some(release.0)
}

/// Whether divider input owned this frame, or ended an old press before a later Down.
pub(super) enum DividerInput {
    Ignored,
    Consumed,
    Handoff,
}

/// Called after wheel/touch navigation, before any art/name press. An existing art press keeps
/// its owner even when it crosses the divider.
pub(super) fn divider_input(
    app: &mut VectorcraftApp,
    ui: &Ui,
    resp: &egui::Response,
    rect: egui::Rect,
    modal: bool,
    space: bool,
    mut after: Option<usize>,
) -> DividerInput {
    let Some(st) = app.session.active() else {
        forget(ui.ctx());
        return DividerInput::Ignored;
    };
    let uid = st.uid;
    let Some((_, view)) = crate::doc_windows::current(app) else { return DividerInput::Ignored };
    let Some(settings) = app.view().map(|v| v.split).filter(|_| enabled(app)) else { return DividerInput::Ignored };
    let held = ui.data(|d| d.get_temp::<DividerDrag>(divider_id()));
    let mut handoff = if let Some(mut held) = held {
        if held.doc != uid || held.view != view || held.orientation != settings.orientation || modal {
            ui.data_mut(|d| d.remove::<DividerDrag>(divider_id()));
            return DividerInput::Ignored;
        }
        let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if escape {
            if let Some(v) = app.view_mut() {
                v.split.fraction = held.start;
            }
            ui.data_mut(|d| d.remove::<DividerDrag>(divider_id()));
            ui.ctx().request_repaint();
            return DividerInput::Consumed;
        }
        let release = primary_release(ui, None);
        move_divider(app, ui, rect, settings.orientation, &mut held.motion, (None, release));
        if release.is_some() || !ui.input(|i| i.pointer.primary_down()) {
            ui.data_mut(|d| d.remove::<DividerDrag>(divider_id()));
        } else {
            ui.data_mut(|d| d.insert_temp(divider_id(), held));
        }
        ui.ctx().request_repaint();
        // A later press is a new gesture, admitted against the divider's final position.
        if !release.is_some_and(|(up, _)| primary_press(ui).is_some_and(|(down, _)| down > up)) {
            return DividerInput::Consumed;
        }
        after = after.max(release.map(|(up, _)| up));
        true
    } else {
        after.is_some()
    };
    // Earlier completed divider pairs apply their own Up before ordinary latest-Down admission.
    if !modal && !crate::touch::finger_navigates_only() {
        after = finish_completed_dividers(app, ui, resp, rect, after, None);
        handoff |= after.is_some();
    }
    let Some(settings) = app.view().map(|v| v.split) else { return DividerInput::Ignored };
    let press = primary_press(ui).filter(|(down, _)| after.is_none_or(|up| *down > up));
    let on = if press.is_some() { press_on_divider(app, ui, resp, rect) } else { on_divider(app, ui, resp, rect) };
    let press_space = press.map_or(space, |(down, _)| drag::primary_press_state(app, ui, down).1);
    if modal || press_space || crate::touch::finger_navigates_only() || !on {
        return if handoff { DividerInput::Handoff } else { DividerInput::Ignored };
    }
    if let Some((index, p)) = press {
        ui.ctx().memory_mut(|mem| mem.stop_text_input());
        app.ui.flyout = None;
        // A completed press applies its own release, never a later hover or another press.
        let mut held = DividerDrag {
            doc: uid,
            view,
            orientation: settings.orientation,
            start: settings.ratio(),
            motion: DividerMotion::new(settings.orientation, p),
        };
        let release = primary_release(ui, Some(index));
        move_divider(app, ui, rect, settings.orientation, &mut held.motion, (Some(index), release));
        if release.is_none() && ui.input(|i| i.pointer.primary_down()) {
            ui.data_mut(|d| d.insert_temp(divider_id(), held));
        }
        if held.motion.moved {
            ui.ctx().request_repaint();
        }
        app.session.tool_press_elsewhere();
    }
    app.session.tool_hover_ended(app.view_info());
    ui.data_mut(|d| d.insert_temp(egui::Id::unique("canvas-hovered"), false));
    DividerInput::Consumed
}

/// Each completed pair is hit-tested against the seam left by the previous pair.
pub(super) fn finish_completed_dividers(
    app: &mut VectorcraftApp,
    ui: &Ui,
    resp: &egui::Response,
    rect: egui::Rect,
    mut after: Option<usize>,
    before: Option<usize>,
) -> Option<usize> {
    while let Some((down, p, up, released)) = completed_divider_press(app, ui, resp, rect, after, before) {
        let Some(settings) = app.view().map(|v| v.split) else { break };
        let mut motion = DividerMotion::new(settings.orientation, p);
        move_divider(app, ui, rect, settings.orientation, &mut motion, (Some(down), Some((up, released))));
        if motion.moved {
            ui.ctx().request_repaint();
        }
        ui.ctx().memory_mut(|mem| mem.stop_text_input());
        app.ui.flyout = None;
        app.session.tool_press_elsewhere();
        after = Some(up);
    }
    after
}

fn divider_axis(orientation: SplitOrientation, p: Pos2) -> f32 {
    match orientation {
        SplitOrientation::Vertical => p.x,
        SplitOrientation::Horizontal => p.y,
    }
}

fn move_divider(
    app: &mut VectorcraftApp,
    ui: &Ui,
    rect: egui::Rect,
    orientation: SplitOrientation,
    motion: &mut DividerMotion,
    interval: (Option<usize>, Option<(usize, Pos2)>),
) {
    let (after, release) = interval;
    // Look only at this owner's samples. A move out and back still starts a drag;
    // a stationary or transverse-only press inside the hit strip does not.
    motion.moved |= ui.input(|i| {
        i.events.iter().enumerate().any(|(index, event)| {
            after.is_none_or(|down| index > down)
                && release.is_none_or(|(up, _)| index <= up)
                && match event {
                    egui::Event::PointerMoved(p) => divider_axis(orientation, *p) != motion.axis,
                    _ => false,
                }
        })
    });
    let Some(p) = release.map(|(_, p)| p).or_else(|| ui.input(|i| i.pointer.interact_pos())) else { return };
    motion.moved |= divider_axis(orientation, p) != motion.axis;
    if !motion.moved {
        return;
    }
    let value = match orientation {
        SplitOrientation::Vertical => (p.x - rect.left()) / rect.width().max(1.0),
        SplitOrientation::Horizontal => (p.y - rect.top()) / rect.height().max(1.0),
    };
    if let Some(v) = app.view_mut() {
        v.split.fraction = value.clamp(0.0, 1.0);
    }
}

pub(super) fn input_view(app: &VectorcraftApp, ui: &Ui, rect: egui::Rect, at: Option<Pos2>) -> ViewInfo {
    let mut view = app.view_info();
    if ui.data(|d| d.get_temp::<Drag>(drag::drag_id()).is_some_and(drag::tool_holds)) {
        if let Some(held) = ui.data(|d| d.get_temp::<HeldMode>(held_id()))
            && app.session.active().is_some_and(|st| st.uid == held.doc)
        {
            view.outline = held.outline;
            view.pixel_preview = held.pixel;
        }
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
