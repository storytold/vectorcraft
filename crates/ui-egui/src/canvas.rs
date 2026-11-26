//! The document canvas: the framework `Canvas` host, its view and the screen controls around it.
//! Everything drawn on or over the document is a seam of its own module; this file builds the
//! canvas, runs the seams in order and copies the view back to the document's `View`.

use craft::behavior::{CanvasView, PressPhase};
use craft::keymap::Mods;
use craft::ui::canvas::{
    Canvas, CanvasScrollbars, CanvasSpace, RotateView, ToolHostResponse, ToolPass, ViewCommands, ViewRequest, ViewTarget, canvas_space, tool_pass,
};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::runtime::env::Env;
use craft::ui::runtime::focus::NudgeMods;
use craft::ui::style::StyleCx;
use egui::Ui;
use serde_json::json;
use vectorcraft_geom::{Rect, Size};

use crate::VectorcraftApp;
use crate::doc_windows::{self, Win};
use crate::state::ViewMove;

mod alternates_bar;
mod artboards;
pub(crate) mod bar_place;
mod context_menu;
mod contour_bar;
mod cursor;
mod drops;
mod fill;
mod grid;
mod guides;
mod home;
pub(crate) mod marks;
mod outline;
mod overlays;
pub(crate) mod panes;
mod quick_pen_bar;
mod raster;
pub(crate) mod rulers;
mod selection;
mod selection_widgets;
pub(crate) mod task_bar;
mod tool_adapter;
mod widgets;

pub use cursor::{HOVER_STILL_SECS, STILL_SECS, ViewKey};
pub use fill::Fill;
use home::home;
pub use panes::{Layout as TileLayout, Panes};
pub(crate) use panes::{consolidate, pane, tile, tiled_area};
pub(crate) use raster::{KeptFrame, color_image, color_image_owned, render_job};
pub(crate) use rulers::{general_lengths, stroke_lengths};
pub use selection::HoverObjectKey;
pub(crate) use task_bar::{TASK_BAR_ACTIONS, TASK_BAR_CHIPS, has_colors, is_task_bar_action, read_hidden_task_bar_actions};
pub use tool_adapter::{apply_requests, dispatch, mods};
pub(crate) use tool_adapter::{busy_tool, in_gesture, pose_on_page, tool_enter};

/// Which body a document's canvas runs: the current window's interactive one, or a display-only
/// pane of a tiled or floating window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BodyKind {
    /// Rulers, scroll bars, bars, tools and drops.
    Live,
    /// The picture, the page marks and nothing that takes a press.
    Pane,
}

/// Performance › Animated Zoom, which needs GPU Performance.
pub(crate) fn animated_zoom(p: &vectorcraft_engine::Prefs) -> bool {
    p.animated_zoom && p.gpu_performance
}

/// Whether a 3D effect is edited in its dialog: the selection's marks and widgets give way to the dialog's gizmo.
pub(crate) fn revolve_editing(app: &VectorcraftApp) -> bool {
    app.three_d_dialog.is_some()
}

/// The arrows the canvas takes with nothing in the window focused, as it takes every arrow while it has the focus:
/// each modifier combination. A plain arrow nudges, Shift ten times as far, Alt nudges a copy, and an arrow an app
/// command is bound to runs it ([`tool_adapter`]).
const ARROWS: NudgeMods = {
    let mut arrows = NudgeMods::NONE;
    let mut held = 0;
    while held < 16 {
        arrows = arrows.with(Mods { cmd: held & 1 != 0, ctrl: held & 2 != 0, alt: held & 4 != 0, shift: held & 8 != 0 });
        held += 1;
    }
    arrows
};

/// The id `Canvas::new("Document")` gets for a body of `win`.
pub(crate) fn canvas_id(win: Win) -> egui::Id {
    egui::Id::unique("Document").with(win)
}

/// Paint the current window's document canvas, or the Home screen, or an empty window.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // A press in the frame that ends a stale pick is the pick's, not the restored tool's, whichever of the frame's
    // passes the tool host gives it in.
    let ended = crate::panels::image_trace::refresh_ignore_sampler(app);
    app.ui.trace_pick_stale = (ended || app.ui.trace_pick_stale) && ui.input(|i| i.pointer.primary_pressed());
    // A release ends the Image Trace panel's eye-button hold, also when the panel is no longer
    // drawn, so a later press elsewhere does not show the source image.
    if app.ui.trace_hold_source.is_some() && !ui.input(|i| i.pointer.any_down()) {
        app.ui.trace_hold_source = None;
    }
    // A document that opened, closed or became active since Home was chosen replaces it.
    if app.ui.home.is_some_and(|k| k != crate::menus::home_key(app)) {
        app.ui.home = None;
    }
    let full = ui.available_rect_before_wrap();
    if crate::menus::home_showing(app) {
        // No type editor shows to lose the keys: the marked text stays as typed.
        crate::type_editing::keep_marked_text(app);
        home(app, ui, full);
        return;
    }
    // A document opened straight through the session has no view yet.
    app.sync_views();
    let Some(win) = doc_windows::current(app).filter(|_| app.session.active().is_some()) else {
        crate::type_editing::keep_marked_text(app);
        // No document and Show The Home Screen When No Documents Are Open off: an empty window.
        let env = Env::of(ui);
        let fill = StyleCx::of_env(ui.ctx(), &env).color(env.tokens.color.surface.canvas);
        ui.painter().rect_filled(full, 0.0, fill.egui());
        return;
    };
    show_document(app, ui, win, BodyKind::Live);
}

/// What the body of one pass hands back to the host.
#[derive(Default)]
struct Body {
    reply: Option<ToolHostResponse>,
    context_at: Option<egui::Pos2>,
    /// A view a compass or scroll bar edit made this pass.
    edited: Option<CanvasView>,
}

/// Window `win`'s document in the framework canvas, filling the rest of `ui`. True when the canvas
/// background got a press this pass, which asks for a pane's window to become the current one.
pub(crate) fn show_document(app: &mut VectorcraftApp, ui: &mut Ui, win: Win, kind: BodyKind) -> bool {
    let area = ui.available_rect_before_wrap();
    let presenting = app.ui.screen_mode == 3;
    let scaling = app.ui_scaling();
    let target = view_target(app, win);
    // A measuring run applies no view request: the view, its request and its fit wait for the pass
    // that shows the canvas.
    let measuring = craft::ui::runtime::measure::active(ui.ctx());
    // Presentation Mode shows the active artboard on the whole screen: it fits again when the screen changes.
    let refits = |v: &crate::state::View| !v.fitted || (presenting && v.rect != area);
    // The view command this pass runs, made for the display the window is on (Display Print Size at 100% Zoom).
    let command = doc_windows::view(app, win)
        .filter(|_| !measuring)
        .and_then(|v| v.request.map(|(r, _)| r.command()).or_else(|| refits(v).then_some(ViewCommands::FitArtboard)));
    if let Some(command) = command {
        app.before_view_command(win, command);
    }
    let ppp = ui.ctx().pixels_per_point();
    let scheme = crate::app::view::zoom_scheme(app.print_ppi_of(Some(win)), ppp / ui.ctx().zoom_factor());
    let Some(view) = doc_windows::view_mut(app, win) else { return false };
    let refit = refits(view);
    let (request, moved) = if measuring {
        (view.request, view.moved)
    } else {
        view.rect = area;
        (view.request.take(), view.moved.take())
    };
    let (mut queued, bounds) = match request {
        Some((request, bounds)) => (vec![request], bounds),
        None => (vec![], None),
    };
    if refit && queued.is_empty() {
        queued.push(ViewRequest::Place(ViewCommands::FitArtboard));
    }
    let mut cv = view.canvas(scaling, area);
    let given = cv;
    // A zoom percentage (Actual Size) counts from the density of the display that shows this
    // window: it runs here with that display's zoom stops, which the canvas's own may not be.
    let mut moved = moved;
    if let Some(request) = queued.first().copied().filter(|r| crate::app::view::fits_window(r.command()) == Some(false)) {
        let layers = move || bounds;
        if request.command().apply(&mut cv, &target, Some(&layers), ppp, &scheme) && matches!(request, ViewRequest::Command(_)) {
            moved = Some(ViewMove::Ease(given));
        }
        queued.clear();
    }
    let (x, y, zero) = rulers::texts(app, win);
    let strips = if app.ui.view.rulers && !presenting {
        Some(rulers::strips(zero, kind))
    } else {
        rulers::hidden(app);
        None
    };
    let pasteboard = app.session.documents().get(win.0).map(|st| artboards::pasteboard_color(app, st, app.outline_view()));
    let layers = move || bounds;
    let prefs = &app.session.prefs;
    let mut canvas = Canvas::new("Document")
        .id(canvas_id(win))
        .view(Bind::bound(&mut cv))
        .document(target)
        .layer_bounds(&layers)
        .queued(&queued)
        .navigation(crate::app::view::navigation(prefs))
        .body_nudges(ARROWS)
        .rulers(x, y)
        .with_rulers(strips)
        .precise_cursors(prefs.use_precise_cursors);
    if let Some(pasteboard) = pasteboard {
        canvas = canvas.pasteboard(pasteboard);
    }
    canvas = match moved {
        Some(ViewMove::Ease(from)) => canvas.ease_from(from, None),
        Some(ViewMove::Jump) => canvas.jumped(),
        None => canvas,
    };
    let mut body = Body::default();
    let shown = canvas.show(ui, |ui, env| {
        if let Some(space) = canvas_space(&env.env) {
            body = show_body(app, ui, &space, tool_pass(&env.env).as_ref(), win, kind);
        }
    });
    let output = shown.output;
    if measuring {
        return false;
    }
    if kind == BodyKind::Live
        && let Some(out) = &output.rulers
    {
        rulers::apply(app, ui, out);
    }
    let pressed = matches!(output.press.phase, PressPhase::Down { .. });
    let Body { reply, context_at, edited } = body;
    let request = reply.as_ref().and_then(|r| r.context.as_ref()).map(|c| c.at).or(context_at);
    let jumped = edited.is_some();
    // Presentation Mode keeps its view through a drag of the middle button, which the canvas pans on its own.
    let view_now =
        if presenting && ui.input(|i| i.pointer.middle_down()) { given } else { edited.or(reply.and_then(|r| r.view)).or(output.view).unwrap_or(cv) };
    if let Some(view) = doc_windows::view_mut(app, win) {
        view.set_canvas(&view_now, scaling);
        // Fitted once, the view stays where it is moved.
        view.fitted |= refit;
        // An edit of the compass or a scroll bar reaches the next pass, which stops a running ease.
        view.moved = jumped.then_some(ViewMove::Jump);
    }
    if let Some(command) = command {
        app.after_view_command(win, command);
    }
    if kind == BodyKind::Pane || presenting {
        return pressed;
    }
    if let Some(space) = output.space.as_ref() {
        context_menu::show(app, ui, space, &output.response, request);
    }
    if let Some(tap) = output.multi_tap
        && app.session.prefs.touch_gestures
        && let Some(cmd) = crate::touch::tap_command(usize::from(tap.fingers), crate::workspaces::is_current(&app.ui, crate::workspaces::TOUCH))
    {
        crate::menus::invoke(app, cmd, json!({}));
    }
    pressed
}

/// The body of the canvas in order: controls that take presses first, the tool host, the picture last.
fn show_body(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace, pass: Option<&ToolPass>, win: Win, kind: BodyKind) -> Body {
    let live = kind == BodyKind::Live;
    let presenting = app.ui.screen_mode == 3;
    let mut out = Body { context_at: artboards::show(app, ui, space, win, kind).map(|p| space.view.to_screen(p)), ..Body::default() };
    grids(app, ui, space, win);
    if live {
        find_shown_tool(app, ui.ctx(), space);
    }
    marks::show(app, ui, space, win, kind);
    if !presenting {
        crate::panels::tabs::stop_guide(app, ui, space);
        crate::dialogs::simplify_slider::show(app, ui, &space.view);
    }
    guides::show(app, ui, space, win, kind);
    selection::show(app, ui, space, win, kind);
    widgets::show(app, ui, space, win, kind);
    if !presenting && !revolve_editing(app) {
        crate::panels::three_d::handles::show(app, ui, space);
    }
    if live {
        if !presenting {
            crate::dialogs::revolve_gizmo::show(app, ui, space);
        }
        selection_widgets::show(app, ui, space, win, kind);
        out.edited = compass(app, ui, space).or_else(|| scroll_bars(app, ui, space));
        if app.ui.screen_mode < 3 {
            let strip = crate::dialogs::simplify_slider::showing(app) || crate::dialogs::smooth_slider::showing(app);
            // The bars give way to a drag, not to a click.
            let dragging = in_gesture(app, ui.ctx()) && ui.input(|i| i.pointer.is_decidedly_dragging());
            let task_bar = (app.ui.task_bar && !revolve_editing(app) && !strip && !dragging).then(|| task_bar::task_bar(app, ui, space)).flatten();
            quick_pen_bar::quick_pen_bar(app, ui, space);
            contour_bar::contour_bar(app, ui, space);
            // After the task bar, which the row of character alternates keeps clear of.
            alternates_bar::alternates_bar(app, ui, space, task_bar);
            crate::free_transform::show(app, ui, space);
            crate::dialogs::dimension::floating(app, ui, space.view.rect);
        }
        let editing = app.session.tool_wants_text();
        let editor = crate::type_editing::show(app, ui);
        out.reply = pass.map(|pass| tool_adapter::run(app, ui, pass));
        if let Some(editor) = editor
            && pass.is_some_and(|pass| matches!(pass.press.phase, PressPhase::Release { .. } | PressPhase::Cancel { .. }))
        {
            crate::type_editing::keep_keys(app, ui.ctx(), editor);
        }
        // An edit the tool host started (a double-click on type) shows its editor in the same pass, so the keys of the
        // next frame are the edit's.
        if !editing {
            crate::type_editing::show(app, ui);
        }
    }
    drops::show(app, ui, win, kind);
    raster::picture(app, ui, space, win, kind);
    out
}

/// The keys held now, as the engine's tools read them, with `space` set while Space is down.
pub(crate) fn held_mods(ctx: &egui::Context) -> vectorcraft_tools::Mods {
    let (m, space) = ctx.input(|i| (i.modifiers, i.key_down(egui::Key::Space)));
    mods(m, space)
}

/// The tool whose marks and widgets the canvas draws with the keys held now: while Cmd is held, the selection tool
/// a press at the pointer borrows ([`find_shown_tool`]), else the active tool.
pub(crate) fn shown_tool(app: &VectorcraftApp, ctx: &egui::Context) -> &'static str {
    app.session.hover_tool_id(&held_mods(ctx))
}

/// Finds the tool whose marks and widgets the canvas draws this frame for the pointer where it is now
/// ([`vectorcraft_engine::Session::hover_tool_at`]), which [`shown_tool`] then answers. While the view moves, the art
/// under the pointer is not hit-tested again (#2452), and the last answer stays.
fn find_shown_tool(app: &mut VectorcraftApp, ctx: &egui::Context, space: &CanvasSpace) {
    let wait = cursor::hover_wait(&mut app.canvas.view_moved, ViewKey::of(&space.view), craft::ui::runtime::frame::clock(ctx));
    if wait.is_some() {
        return;
    }
    let held = held_mods(ctx);
    let named = app.session.shown_tool_id(&held);
    let at = ctx.input(|i| i.pointer.hover_pos()).filter(|p| space.view.rect.contains(*p)).map(|p| space.view.to_doc(p));
    let view = app.view_info();
    app.session.hover_tool_at(at, &held, view);
    // The status bar and the help bar are drawn before the canvas: when the tool they name changes with this
    // frame's pointer, they are drawn again.
    if app.session.shown_tool_id(&held) != named {
        ctx.request_repaint();
    }
}

/// The document grid, in the picture slot under the art when Grids In Back is on and over it
/// otherwise, and the pixel grid over the art.
fn grids(app: &VectorcraftApp, ui: &mut Ui, space: &CanvasSpace, win: Win) {
    if let Some((grid, in_back)) = app.session.documents().get(win.0).and_then(|st| grid::document_grid(app, st)) {
        let _ = if in_back { grid.picture() } else { grid }.show_at(ui, space.view.rect);
    }
    if let Some(grid) = grid::pixel_grid(app, app.print_ppi_of(Some(win))) {
        let _ = grid.show_at(ui, space.view.rect);
    }
}

/// The Rotate View compass while the view is turned: the view with the angle the user set.
fn compass(app: &VectorcraftApp, ui: &mut Ui, space: &CanvasSpace) -> Option<CanvasView> {
    if app.ui.screen_mode >= 3 || !RotateView::visible(space.view.rotation) {
        return None;
    }
    let mut turn = space.view.rotation;
    let rect = RotateView::compass_rect(ui, space.view.rect);
    RotateView::new(tl!("Rotate View"), Bind::bound(&mut turn), false).show_at(ui, rect);
    (turn != space.view.rotation).then(|| {
        let mut view = space.view;
        view.rotation = turn;
        view
    })
}

/// The scroll bars along the right and bottom edges (Normal and Full Screen with Menu Bar): the view
/// a thumb drag or page press moved.
fn scroll_bars(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace) -> Option<CanvasView> {
    if app.ui.screen_mode >= 2 {
        return None;
    }
    let visible = space.view.visible();
    let extent = app.content_bounds()?.inflate(visible.width() / 2.0, visible.height() / 2.0);
    let mut bar_view = space.view;
    CanvasScrollbars::new(tl!("Scroll"), Bind::bound(&mut bar_view), extent).show_at(ui, space.view.rect);
    (bar_view != space.view).then_some(bar_view)
}

/// The size of the art and the artboards together, and the artboard Fit Artboard shows (the view's,
/// the first when it has gone).
pub(crate) fn view_target(app: &VectorcraftApp, win: Win) -> ViewTarget {
    let (Some(st), Some(view)) = (app.session.documents().get(win.0), doc_windows::view(app, win)) else {
        return ViewTarget::new(Size::new(1.0, 1.0));
    };
    let boards = st.doc.artboards.iter().map(|a| a.rect).reduce(|r, a| r.union(a));
    let target = ViewTarget::new(document_size(vectorcraft_geom::union_opt(st.doc.art_bounds(), boards)));
    match st.doc.artboards.get(view.artboard).or(st.doc.artboards.first()) {
        Some(a) => target.artboard(a.rect),
        None => target,
    }
}

/// The size the view fits and an autoscroll stops at: the art and the artboards together (`all`), at least 1 × 1.
pub(crate) fn document_size(all: Option<Rect>) -> Size {
    all.map_or(Size::new(1.0, 1.0), |r| Size::new(r.width().max(1.0), r.height().max(1.0)))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_edge_scroll;
#[cfg(test)]
mod tests_ui_scaling;
