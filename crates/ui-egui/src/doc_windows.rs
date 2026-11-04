//! Window › Arrange › Float in Window, Float All in Windows and Cascade: documents in floating
//! windows of their own, out of the tab strip, until Consolidate All Windows. On the desktop each
//! window is an OS window; on the web, on Wayland and wherever egui draws every viewport inside
//! the main window, it is a window inside the app's window, as floating panels are
//! ([`crate::panel_docking`]). The current document's window holds the canvas
//! ([`crate::canvas::show`]); each other window draws its document as a Tile pane does
//! ([`crate::canvas::pane`]), and a press in it makes its document the current one.
//!
//! A document tab dragged off the tab strip floats its document in a window under the pointer,
//! and a document window dragged onto the tab strip by its title bar (an OS window by the tab at
//! its top) goes back under the tabs.
//!
//! With Preferences › User Interface › Open Documents As Tabs cleared, each document opened or
//! created afterwards opens in a floating window ([`opened`]).
//!
//! Each view of a document that Window › New Window opened ([`views`]) is a window of its own
//! here: it has its own tab, pane or floating window.

use serde_json::{Value, json};

use crate::VectorcraftApp;

mod views;
pub(crate) use views::{
    Views, Win, close_current, close_window, current, label, new_window, retain as retain_views, show_window, unfit_all, view, view_mut, windows,
};
use views::{find, key, window_param};

/// How far above or below the tab strip, in points, a dragged document tab must be released to
/// float its document.
pub(crate) const TEAR_OFF: f32 = 24.0;
/// Where a torn-off document's window opens: this far up and left of the pointer, so the pointer
/// is over its title bar.
pub(crate) const GRAB: egui::Vec2 = egui::vec2(40.0, 12.0);
/// How far apart Cascade sets the windows, down and to the right, in points.
pub(crate) const CASCADE_STEP: f32 = 30.0;
/// How far the first cascaded window is from the document area's top-left corner, in points.
const CASCADE_MARGIN: f32 = 24.0;
/// The smallest size of a document window, in points.
const MIN_SIZE: egui::Vec2 = egui::vec2(240.0, 180.0);

/// A document's view in a floating window.
#[derive(Clone, Debug)]
pub struct DocWindow {
    /// The document ([`vectorcraft_engine::DocState::uid`]).
    pub uid: u64,
    /// The view's number ([`views`]).
    pub view: u32,
    /// Where the window goes, in the main window's points.
    pub place: egui::Rect,
    /// The window's own number ([`Windows::opened`]): a document floated again gets a new window,
    /// which opens where it is asked to, not where its last window was.
    number: u32,
    /// Counts the moves Cascade gives the window after it opened.
    generation: u32,
    /// The OS window's position on the screen when it opened or last moved, and the generation of
    /// that move. Repeated each frame, so egui sees an unchanged window.
    screen: Option<(egui::Pos2, u32)>,
    /// Its OS window had the keyboard focus in the last frame.
    focused: bool,
    /// Where the last frame drew its document, in its window's coordinates.
    pub(crate) body: Option<egui::Rect>,
    /// The size of the body its document was last fitted to, and the zoom and center that fit
    /// gave: a window resized while its view is still there fits its document again.
    fit: Option<Fit>,
    /// While its OS window's tab is dragged: where the pointer is from the window's top-left
    /// corner, on the screen.
    grab: Option<egui::Vec2>,
}

/// [`DocWindow::fit`]: a body size, and a view's zoom and center.
type Fit = (egui::Vec2, f64, vectorcraft_geom::Point);

impl DocWindow {
    fn new((uid, view): (u64, u32), place: egui::Rect) -> Self {
        Self { uid, view, place, number: 0, generation: 0, screen: None, focused: false, body: None, fit: None, grab: None }
    }
}

/// What the document windows keep between frames.
#[derive(Default)]
pub struct Windows {
    /// The document area of the main window (the tabs and the canvas) in its last frame.
    pub(crate) area: Option<egui::Rect>,
    /// The current window ([`views::key`]) when the windows were last drawn: a floating window
    /// that becomes the current one comes to the front.
    current: Option<(u64, u32)>,
    /// While Presentation Mode is on: the floating windows ([`views::key`]) it has shown.
    presented: Option<Vec<(u64, u32)>>,
    /// The main window's tab strip in its last frame, while it was drawn.
    pub(crate) strip: Option<egui::Rect>,
    /// Counts the windows opened ([`DocWindow::number`]).
    opened: u32,
    /// The open documents (their uids) when [`opened`] last ran, or when the app started.
    known: Vec<u64>,
    /// The documents' views beyond the one each shows ([`views`]).
    pub(crate) views: Views,
}

impl Windows {
    /// No windows yet, with `known` (uids) the documents open when the app starts: they keep their
    /// tabs, whatever Open Documents As Tabs says.
    pub(crate) fn new(known: Vec<u64>) -> Self {
        Self { known, ..Default::default() }
    }
}

/// Whether a document tab dragged from `strip` to `p` has left it far enough to float.
pub(crate) fn torn_off(strip: egui::Rect, p: egui::Pos2) -> bool {
    p.y > strip.bottom() + TEAR_OFF || p.y < strip.top() - TEAR_OFF
}

/// Whether window `w` is in a floating window.
pub(crate) fn floats(app: &VectorcraftApp, w: Win) -> bool {
    key(app, w).is_some_and(|(u, n)| app.ui.floating.iter().any(|x| x.uid == u && x.view == n))
}

/// Whether document `i`'s shown view is in a floating window.
#[cfg(test)]
pub(crate) fn is_floating(app: &VectorcraftApp, i: usize) -> bool {
    floats(app, (i, views::shown(app, i)))
}

/// The windows under the tab strip, in tab order.
pub(crate) fn docked_windows(app: &VectorcraftApp) -> Vec<Win> {
    windows(app).into_iter().filter(|w| !floats(app, *w)).collect()
}

/// The documents with a window under the tab strip, by index, in tab order.
#[cfg(test)]
pub(crate) fn docked(app: &VectorcraftApp) -> Vec<usize> {
    let mut docs: Vec<usize> = docked_windows(app).into_iter().map(|(i, _)| i).collect();
    docs.dedup();
    docs
}

/// The window the tab strip shows: the current one when it is under the tabs, else the tabbed
/// window that was current last, else the first tabbed one.
pub(crate) fn tab_window(app: &VectorcraftApp) -> Option<Win> {
    let docked = docked_windows(app);
    if let Some(c) = current(app).filter(|c| docked.contains(c)) {
        return Some(c);
    }
    let last = app.ui.tab_document.and_then(|k| find(app, k));
    last.filter(|w| docked.contains(w)).or(docked.first().copied())
}

/// The document of the window the tab strip shows ([`tab_window`]).
#[cfg(test)]
pub(crate) fn tab_document(app: &VectorcraftApp) -> Option<usize> {
    tab_window(app).map(|(i, _)| i)
}

/// Whether the current window is a floating one, outside the Home screen and Presentation Mode:
/// the main window's canvas then draws the tab strip's window as a pane.
pub(crate) fn current_floats(app: &VectorcraftApp) -> bool {
    current(app).is_some_and(|w| floats(app, w)) && !crate::menus::home_showing(app) && !presenting(app)
}

/// Whether Presentation Mode is on: the main window shows the current document, and the floating
/// windows are hidden until it ends.
fn presenting(app: &VectorcraftApp) -> bool {
    app.ui.screen_mode >= 3
}

/// The places Cascade gives `n` windows in `area`: each the same size, the first near the area's
/// top-left corner and each next one [`CASCADE_STEP`] further down and to the right, starting
/// again at the corner when the next one would leave the area. Every place lies inside `area`.
pub(crate) fn cascade_places(area: egui::Rect, n: usize) -> Vec<egui::Rect> {
    let size = (area.size() * 0.6).max(MIN_SIZE).min(area.size());
    let room = (area.size() - size).min_elem().max(0.0);
    let steps = if room > CASCADE_MARGIN { ((room - CASCADE_MARGIN) / CASCADE_STEP).floor() as usize + 1 } else { 1 };
    (0..n)
        .map(|k| {
            let offset = (CASCADE_MARGIN + (k % steps) as f32 * CASCADE_STEP).min(room);
            egui::Rect::from_min_size(area.min + egui::vec2(offset, offset), size)
        })
        .collect()
}

/// The document area windows open in: the main window's last one, else a 1200 × 800 area.
fn area(app: &VectorcraftApp) -> egui::Rect {
    app.canvas.windows.area.filter(|r| r.is_positive()).unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0)))
}

/// Float windows `wins`, each at the next cascade place after the windows already open. Each fits
/// its window.
fn float(app: &mut VectorcraftApp, wins: &[Win]) {
    app.sync_views();
    place(app, wins);
}

/// The documents opened or created since the last call: each one opened while Preferences › User
/// Interface › Open Documents As Tabs was cleared ([`vectorcraft_engine::DocState::opened_as_tab`])
/// opens in a floating window of its own, at the next cascade place. Documents already open stay
/// where they are. Called by [`VectorcraftApp::sync_views`].
pub(crate) fn opened(app: &mut VectorcraftApp) {
    let docs = app.session.documents();
    let known = &app.canvas.windows.known;
    if known.len() == docs.len() && known.iter().zip(docs).all(|(u, d)| *u == d.uid) {
        return;
    }
    let new: Vec<Win> = docs.iter().enumerate().filter(|(_, d)| !d.opened_as_tab && !known.contains(&d.uid)).map(|(i, _)| (i, 1)).collect();
    app.canvas.windows.known = docs.iter().map(|d| d.uid).collect();
    if !new.is_empty() {
        place(app, &new);
    }
}

/// [`float`] for windows whose views exist.
fn place(app: &mut VectorcraftApp, wins: &[Win]) {
    let first = app.ui.floating.len();
    let places = cascade_places(area(app), first + wins.len());
    for (k, w) in wins.iter().enumerate() {
        let Some(key) = key(app, *w) else { continue };
        if floats(app, *w) {
            continue;
        }
        let place = places.get(first + k).copied().unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, MIN_SIZE));
        app.canvas.windows.opened = app.canvas.windows.opened.wrapping_add(1);
        let mut win = DocWindow::new(key, place);
        win.number = app.canvas.windows.opened;
        app.ui.floating.push(win);
        if let Some(v) = view_mut(app, *w) {
            v.fitted = false;
        }
    }
    if !wins.is_empty() {
        app.ui.home = None;
    }
}

/// The floating windows' document indices, in the order the windows opened.
fn floating_indices(app: &VectorcraftApp) -> Vec<usize> {
    floating_windows(app).into_iter().map(|(i, _)| i).collect()
}

/// The floating windows, in the order they opened.
pub(crate) fn floating_windows(app: &VectorcraftApp) -> Vec<Win> {
    app.ui.floating.iter().filter_map(|w| find(app, (w.uid, w.view))).collect()
}

/// `window.arrange.floatInWindow`: the current window, or document `index`'s view `view` (its
/// shown view without one), leaves the tab strip for a floating window of its own, with its
/// top-left corner at `at` (`[x, y]` in the main window's points) when given, else at the next
/// cascade place. A window already floating stays where it is. The window `index` names becomes
/// the current one. Returns `{floating}`, the floating windows' document indices.
pub(crate) fn float_in_window(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let named = window_param(app, p)?;
    let at = p
        .get("at")
        .and_then(Value::as_array)
        .and_then(|a| Some(egui::pos2(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)))
        .filter(|at| at.is_finite());
    if let Some(w) = named.or(current(app)).filter(|w| !floats(app, *w)) {
        float(app, &[w]);
        if let (Some(at), Some(w)) = (at, app.ui.floating.last_mut()) {
            w.place = egui::Rect::from_min_size(at, w.place.size());
        }
    }
    if let Some(w) = named {
        show_window(app, w);
    }
    Ok(json!({ "floating": floating_indices(app) }))
}

/// `window.arrange.consolidateAllWindows` with an `index`: document `index`'s view `view` (its
/// shown view without one), floating or not, goes back under its tab, fitted there, and becomes
/// the current window; the other windows and Tile stay as they are. Returns `{tiled, floating}`.
pub(crate) fn dock(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let Some(w) = window_param(app, p)? else {
        return Err("no such document".into());
    };
    if let Some((u, n)) = key(app, w).filter(|_| floats(app, w)) {
        app.ui.floating.retain(|x| !(x.uid == u && x.view == n));
        if let Some(v) = view_mut(app, w) {
            v.fitted = false;
        }
    }
    show_window(app, w);
    Ok(json!({ "tiled": app.ui.tiled, "floating": floating_indices(app) }))
}

/// `window.arrange.floatAllInWindows`: every window under the tab strip goes to a floating window
/// of its own. Returns `{floating}`.
pub(crate) fn float_all(app: &mut VectorcraftApp) -> Result<Value, String> {
    let docked = docked_windows(app);
    float(app, &docked);
    Ok(json!({ "floating": floating_indices(app) }))
}

/// `window.arrange.cascade`: every window floating, stacked from the document area's upper left to
/// its lower right in tab order, each fitted to its window, the current window in front. Returns
/// `{floating}`.
pub(crate) fn cascade(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.ui.tiled = false;
    let docked = docked_windows(app);
    float(app, &docked);
    let order: Vec<(u64, u32)> = windows(app).into_iter().filter_map(|w| key(app, w)).collect();
    app.ui.floating.sort_by_key(|w| order.iter().position(|k| *k == (w.uid, w.view)));
    let places = cascade_places(area(app), app.ui.floating.len());
    for (w, place) in app.ui.floating.iter_mut().zip(places) {
        w.place = place;
        w.generation += 1;
    }
    unfit_all(app);
    // The current window comes to the front.
    app.canvas.windows.current = None;
    Ok(json!({ "floating": floating_indices(app) }))
}

/// Every floating window back under the tab strip ([`crate::canvas::consolidate`], Tile), each
/// fitted to where it shows.
pub(crate) fn dock_all(app: &mut VectorcraftApp) {
    for w in floating_windows(app) {
        if let Some(v) = view_mut(app, w) {
            v.fitted = false;
        }
    }
    app.ui.floating.clear();
}

/// Whether the floating windows are OS windows: egui runs viewports of their own, the session is
/// not Wayland, where the main window cannot learn where its other windows are (craft-ui's docking
/// floats panels by the same rule), and no control channel is attached. With a control channel the
/// windows stay inside the main window, where `ui.click`, `ui.drag` and `ui.screenshot` reach them
/// and `canvasRect` is in the main window's coordinates.
fn native(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    if ctx.embed_viewports() || app.control_rx.is_some() {
        return false;
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        if set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET") {
            // Under XWayland the main window reports where it is.
            return ctx.input_for(egui::ViewportId::ROOT, |i| i.viewport().outer_rect.is_some());
        }
    }
    true
}

/// The OS window of document window `w`.
fn viewport_id(w: &DocWindow) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("vectorcraft-document-window", w.uid, w.number))
}

/// The id of document window `w` inside the main window: a new one for each window and each
/// Cascade move, so egui opens it at its new place and size.
fn window_id(w: &DocWindow) -> egui::Id {
    egui::Id::unique(("vectorcraft-document-window", w.uid, w.number, w.generation))
}

/// What happened in a document window this frame.
#[derive(Default)]
struct Shown {
    /// Its close button was pressed.
    close: bool,
    /// Its document asks to become the current one.
    activate: bool,
    /// It is being dragged over the main window's tab strip.
    over_strip: bool,
    /// It was dropped on the main window's tab strip: its document goes back under the tabs.
    dock: bool,
    /// Its OS window has the keyboard focus.
    focused: bool,
    /// Where it drew its document.
    body: Option<egui::Rect>,
    /// [`DocWindow::fit`] after this frame.
    fit: Option<Fit>,
}

/// Draw the floating document windows, after the main window's canvas. Call once per frame from
/// the main window.
pub(crate) fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let live: Vec<(u64, u32)> = windows(app).into_iter().filter_map(|w| key(app, w)).collect();
    app.ui.floating.retain(|w| live.contains(&(w.uid, w.view)));
    let current = current(app).and_then(|w| key(app, w));
    let floating = |app: &VectorcraftApp, k: &(u64, u32)| app.ui.floating.iter().any(|w| (w.uid, w.view) == *k);
    if let Some(k) = current.filter(|k| !floating(app, k)) {
        app.ui.tab_document = Some(k);
    }
    // Presentation Mode fits each floating window it shows to the main window: back in their
    // floating windows, those views fit their windows again.
    let presenting = presenting(app);
    if presenting {
        let is_floating = current.is_some_and(|k| floating(app, &k));
        let shown = app.canvas.windows.presented.get_or_insert_default();
        if let Some(k) = current.filter(|k| is_floating && !shown.contains(k)) {
            shown.push(k);
            if let Some(v) = app.session.active_index().and_then(|i| app.views.get_mut(i)) {
                v.fitted = false;
                ctx.request_repaint();
            }
        }
    } else if let Some(shown) = app.canvas.windows.presented.take() {
        let wins: Vec<Win> = shown.into_iter().filter_map(|k| find(app, k)).collect();
        for w in wins {
            if let Some(v) = view_mut(app, w) {
                v.fitted = false;
            }
        }
    }
    // While Presentation Mode hides the windows, `current` keeps the window that was in front, so a
    // window made current meanwhile comes to the front when it ends.
    if presenting {
        return;
    }
    if app.ui.floating.is_empty() {
        app.canvas.windows.current = current;
        return;
    }
    let raise = current.filter(|u| app.canvas.windows.current != Some(*u));
    let native = native(app, ctx);
    // Windows inside the main window that come back (after Presentation Mode) all go to the front
    // in their old order, so the raise waits for the next frame.
    let returning = !native
        && app.ui.floating.iter().any(|w| {
            let layer = egui::LayerId::new(egui::Order::Middle, window_id(w));
            ctx.memory(|m| m.area_rect(layer.id).is_some() && !m.areas().visible_last_frame(&layer))
        });
    if raise.is_some() && returning {
        ctx.request_repaint();
    } else {
        app.canvas.windows.current = current;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if native {
        input::register(ctx, app.services.native_input_hook.clone(), app.ui.floating.iter().map(viewport_id).collect());
    }
    // While a dialog or other modal is open the windows take no input, as the main window behind
    // it takes none: each draws its document as a pane, the current one included.
    let blocked = crate::panel_docking::modal_open(app, ctx);
    let mut close = None;
    let mut activate = None;
    let mut dock = None;
    let mut over_strip = false;
    for w in app.ui.floating.clone() {
        let Some(win) = find(app, (w.uid, w.view)) else { continue };
        let raised = raise == Some((w.uid, w.view));
        let shown = if native { os_window(app, ctx, &w, (win, blocked), raised) } else { app_window(app, ctx, &w, (win, blocked), raised) };
        if blocked {
            continue;
        }
        let is_current = views::current(app) == Some(win);
        if let Some(x) = app.ui.floating.iter_mut().find(|x| x.uid == w.uid && x.view == w.view) {
            // The OS window took the keyboard: its view becomes the current window.
            if shown.focused && !x.focused && !is_current {
                activate = Some(win);
            }
            x.focused = shown.focused;
            x.body = shown.body;
            x.fit = shown.fit;
        }
        over_strip |= shown.over_strip;
        if shown.close {
            close = Some(win);
        } else if shown.dock {
            dock = Some(win);
        } else if shown.activate && !is_current {
            activate = Some(win);
        }
    }
    // The tab strip a dragged window would go back under shows where it lands.
    if let Some(strip) = app.canvas.windows.strip.filter(|_| over_strip) {
        let t = crate::theme::Tokens::get(ctx);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("vectorcraft-document-dock")));
        painter.rect_filled(strip, 0.0, t.accent.gamma_multiply(0.12));
        painter.rect_stroke(strip, 0.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    if let Some(win) = close {
        // The save prompt opens in the main window.
        if let Err(e) = close_window(app, win) {
            app.status(e);
        }
        if native && app.ui.dialog.is_some() {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
    } else if let Some((i, n)) = dock {
        if let Err(e) = app.run("window.arrange.consolidateAllWindows", json!({ "index": i, "view": n })) {
            app.status(e);
        }
        if native {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
    } else if let Some(win) = activate {
        show_window(app, win);
        app.canvas.windows.current = views::current(app).and_then(|w| key(app, w));
    }
}

/// Window `win`'s body in `ui`: the canvas for the current window, else (and while `blocked` by a
/// modal) a pane. The view fits the body again when the body changed size since `fit` and the view
/// has not moved. → whether a press in the pane asks for its window to become the current one, the
/// body's rect and the fit after this frame.
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, (win, blocked): (Win, bool), fit: Option<Fit>) -> (bool, egui::Rect, Option<Fit>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, 0.0, t.pasteboard);
    if let (Some(v), Some((size, zoom, center))) = (view_mut(app, win), fit)
        && size != rect.size()
        && zoom == v.zoom
        && center == v.center
    {
        v.fitted = false;
    }
    let was_fitted = view(app, win).is_some_and(|v| v.fitted);
    let pressed = if !blocked && views::current(app) == Some(win) && !crate::menus::home_showing(app) {
        crate::canvas::show(app, ui);
        false
    } else {
        crate::canvas::pane(app, ui, win, rect)
    };
    ui.advance_cursor_after_rect(rect);
    let fit = match view(app, win) {
        Some(v) if v.fitted && !was_fitted => Some((rect.size(), v.zoom, v.center)),
        _ => fit,
    };
    (pressed, rect, fit)
}

/// Window `win`'s floating window `w` as an OS window of its own, `blocked` by a modal or not;
/// `raise` brings it to the front. Its tab is at the top of the window, above its body.
fn os_window(app: &mut VectorcraftApp, ctx: &egui::Context, w: &DocWindow, (win, blocked): (Win, bool), raise: bool) -> Shown {
    let vid = viewport_id(w);
    let origin = ctx.input(|inp| inp.viewport().inner_rect.map(|r| r.min)).filter(|p| p.is_finite()).unwrap_or(egui::Pos2::ZERO);
    let screen = match w.screen {
        Some((pos, generation)) if generation == w.generation => pos,
        moved => {
            let pos = origin + w.place.min.to_vec2();
            if moved.is_some() {
                ctx.send_viewport_cmd_to(vid, egui::ViewportCommand::OuterPosition(pos));
                ctx.send_viewport_cmd_to(vid, egui::ViewportCommand::InnerSize(w.place.size()));
            }
            if let Some(x) = app.ui.floating.iter_mut().find(|x| x.uid == w.uid && x.view == w.view) {
                x.screen = Some((pos, w.generation));
            }
            pos
        }
    };
    if raise {
        ctx.send_viewport_cmd_to(vid, egui::ViewportCommand::Focus);
    }
    let title = crate::chrome::document_window_title(app, win).unwrap_or_default();
    let builder =
        egui::ViewportBuilder::default().with_title(title).with_position(screen).with_inner_size(w.place.size()).with_min_inner_size(MIN_SIZE);
    let mut forwarded = (Vec::new(), None);
    // The main window's tab strip on the screen.
    let strip = app.canvas.windows.strip.map(|s| s.translate(origin.to_vec2()));
    let shown = ctx.show_viewport_immediate(vid, builder, |ui, _| {
        let (close, focused) = ui.input(|inp| (inp.viewport().close_requested(), inp.viewport().focused == Some(true)));
        if close {
            // The OS window stays until its document closes, which stops drawing it: a save prompt
            // or a modal can keep the document open.
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        let tab = crate::chrome::window_tab(app, ui, win);
        let (over_strip, dock) = if blocked { (false, false) } else { drag_tab(app, ui, ((w.uid, w.view), origin), &tab, strip) };
        let (pressed, rect, fit) = body(app, ui, (win, blocked), w.fit);
        let activate = pressed || (!blocked && (tab.clicked() || tab.drag_started()));
        // Keys the window's widgets left over run the main window's shortcuts and tool keys, as a
        // floating panel's do.
        if !blocked && !ui.memory(|m| m.focused().is_some()) {
            forwarded = ui.input(|inp| {
                let events: Vec<egui::Event> = inp
                    .events
                    .iter()
                    .filter(|e| {
                        matches!(e, egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))
                    })
                    .cloned()
                    .collect();
                (events, Some(inp.modifiers))
            });
        }
        Shown { close, activate, over_strip, dock, focused, body: Some(rect), fit }
    });
    if !forwarded.0.is_empty() {
        crate::panel_docking::forward_keys(app, ctx, forwarded.0, forwarded.1);
    }
    shown
}

/// A drag of floating window `key`'s ([`views::key`]) OS window by the `tab` at its top: the window
/// follows the pointer, and its view goes back under the tabs when the tab is released over the
/// main window's tab `strip` (on the screen). `origin` is the main window's top-left corner on the
/// screen. → whether the pointer is over the strip, and whether the tab was released there.
fn drag_tab(
    app: &mut VectorcraftApp,
    ui: &egui::Ui,
    ((uid, view), origin): ((u64, u32), egui::Pos2),
    tab: &egui::Response,
    strip: Option<egui::Rect>,
) -> (bool, bool) {
    let Some(win) = app.ui.floating.iter_mut().find(|x| x.uid == uid && x.view == view) else {
        return (false, false);
    };
    if !tab.dragged() && !tab.drag_stopped() {
        win.grab = None;
        return (false, false);
    }
    let (inner, outer, pointer) = ui.input(|inp| (inp.viewport().inner_rect, inp.viewport().outer_rect, inp.pointer.latest_pos()));
    let (Some(inner), Some(outer), Some(pointer)) = (inner, outer, pointer) else {
        return (false, false);
    };
    let at = inner.min + pointer.to_vec2();
    // The pointer stays where it pressed the tab, relative to the window.
    let pressed = ui.input(|inp| inp.pointer.press_origin()).map_or(at, |p| inner.min + p.to_vec2());
    let grab = *win.grab.get_or_insert(pressed - outer.min);
    let over = strip.is_some_and(|s| s.contains(at));
    if tab.drag_stopped() {
        win.grab = None;
        return (over, over);
    }
    let pos = at - grab;
    if pos != outer.min {
        win.screen = Some((pos, win.generation));
        win.place = egui::Rect::from_min_size(pos - origin.to_vec2(), win.place.size());
    }
    (over, false)
}

/// Window `win`'s floating window `w` as a window inside the main window, `blocked` by a modal or
/// not; `raise` brings it to the front.
fn app_window(app: &mut VectorcraftApp, ctx: &egui::Context, w: &DocWindow, (win, blocked): (Win, bool), raise: bool) -> Shown {
    let id = window_id(w);
    if raise {
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
    }
    let title = crate::chrome::document_window_title(app, win).unwrap_or_default();
    let mut open = true;
    let (mut activate, mut rect, mut fit) = (false, None, w.fit);
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(0.0);
    let shown = egui::Window::new(title)
        .id(id)
        .default_rect(w.place)
        .min_size(MIN_SIZE)
        .collapsible(false)
        .resizable(true)
        .constrain_to(ctx.content_rect())
        .frame(frame)
        .open(&mut open)
        .show(ctx, |ui| {
            let (pressed, r, f) = body(app, ui, (win, blocked), fit);
            (activate, rect, fit) = (pressed, Some(r), f);
        });
    // A press on its title bar or edges, which brings the window to the front, also makes its
    // document the current one.
    activate |= shown.as_ref().is_some_and(|s| s.response.contains_pointer()) && ctx.input(|inp| inp.pointer.any_pressed());
    // Moved by its title bar onto the main window's tab strip, it goes back under the tabs.
    // egui moves a window by the drag of its title bar's own widget, or, where it moves windows by
    // a drag anywhere (after touch input), by the drag of the window itself.
    let mover = ctx.read_response(id.with("__title_click")).or_else(|| shown.as_ref().map(|s| s.response.clone()));
    let moved = mover.map_or((false, false), |r| (r.dragged(), r.drag_stopped()));
    let pointer = ctx.input(|inp| inp.pointer.latest_pos());
    let over_strip = !blocked && (moved.0 || moved.1) && app.canvas.windows.strip.zip(pointer).is_some_and(|(strip, p)| strip.contains(p));
    Shown { close: !open, activate, over_strip, dock: over_strip && moved.1, focused: false, body: rect, fit }
}

/// The host's rewrites of raw input ([`crate::services::Services::native_input_hook`]) for the
/// document windows' OS windows: eframe runs the host's hook for the main window only.
#[cfg(not(target_arch = "wasm32"))]
mod input {
    use std::collections::HashSet;

    use crate::services::NativeInputHook;

    /// Runs the hook on the input of the document windows' OS windows.
    #[derive(Default)]
    struct DocumentWindowInput {
        hook: Option<NativeInputHook>,
        windows: HashSet<egui::ViewportId>,
    }

    impl egui::Plugin for DocumentWindowInput {
        fn debug_name(&self) -> &'static str {
            "vectorcraft document windows input"
        }

        fn input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
            if let Some(hook) = self.hook.as_ref().filter(|_| self.windows.contains(&input.viewport_id)) {
                hook(input);
            }
        }
    }

    /// Run `hook` on the input of `windows` from their next frame on.
    pub(super) fn register(ctx: &egui::Context, hook: Option<NativeInputHook>, windows: HashSet<egui::ViewportId>) {
        if hook.is_none() && ctx.with_plugin(|_: &mut DocumentWindowInput| ()).is_none() {
            return;
        }
        ctx.add_plugin(DocumentWindowInput::default());
        ctx.with_plugin(|p: &mut DocumentWindowInput| {
            p.hook = hook;
            p.windows = windows;
        });
    }
}

#[cfg(test)]
mod tests;
