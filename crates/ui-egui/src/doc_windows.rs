//! Window › Arrange › Float in Window, Float All in Windows and Cascade: documents in floating
//! windows of their own, out of the tab strip, until Consolidate All Windows. Each is a
//! [`DocumentWindow`]: an OS window on the desktop, a window inside the app's window on the web, on
//! Wayland and wherever the framework draws every viewport inside the main window. The current
//! document's window holds the canvas ([`crate::canvas::show_document`]); each other window draws
//! its document as a Tile pane does ([`crate::canvas::pane`]), and a press in it makes its
//! document the current one.
//!
//! A document tab dragged off the tab strip floats its document in a window under the pointer
//! (`DocumentTabs` reports the tear-off), and a window's own tab dropped on the main strip puts it
//! back under the tabs ([`crate::chrome`]).
//!
//! With Preferences › User Interface › Open Documents As Tabs cleared, each document opened or
//! created afterwards opens in a floating window ([`opened`]).
//!
//! Each view of a document that Window › New Window opened ([`views`]) is a window of its own
//! here: it has its own tab, pane or floating window.

use craft::ui::contract::{Props as _, ShowContainer as _};
use craft::ui::runtime::windows::WindowHost;
use craft::ui::shell::DocumentWindow;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::canvas::{BodyKind, show_document};

mod views;
pub(crate) use views::{
    Views, Win, close_current, close_window, current, label, new_window, retain as retain_views, show_window, unfit_all, view, view_mut, windows,
};
use views::{find, key, window_param};

/// Where a torn-off document's window opens: this far up and left of the pointer, so the pointer
/// is over its title bar.
pub(crate) const GRAB: egui::Vec2 = egui::vec2(40.0, 12.0);
/// How far apart Cascade sets the windows, down and to the right, in points.
pub(crate) const CASCADE_STEP: f32 = 30.0;
/// How far the first cascaded window sits from the document area's top-left corner, clear of the
/// tab strip.
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
    /// Its OS window had the keyboard focus in the last frame.
    focused: bool,
    /// The density of the display that shows its OS window ([`crate::services::screen_ppi_at`]),
    /// while the window is on a display the host listed; `None` before that, while it is on none
    /// of them, and inside the main window, where the window counts the main window's density.
    ppi: Option<crate::ScreenPpi>,
    /// The last frame drew it as an OS window of its own.
    os: bool,
    /// A display that shows its OS window has been read since it opened.
    was_read: bool,
}

impl DocWindow {
    fn new((uid, view): (u64, u32), place: egui::Rect) -> Self {
        Self { uid, view, place, number: 0, generation: 0, focused: false, ppi: None, os: false, was_read: false }
    }
}

/// The density of the display that shows floating window `x`'s OS window: read once the window
/// was found on a display, unread until then; `None` for a window inside the main window, which
/// counts the main window's density, and for the window Presentation Mode shows there.
fn window_ppi(app: &VectorcraftApp, x: &DocWindow) -> Option<crate::ScreenPpi> {
    if presenting(app) && current(app).and_then(|w| key(app, w)) == Some((x.uid, x.view)) {
        return None;
    }
    x.ppi.or(x.os.then_some(crate::ScreenPpi::Unread))
}

/// [`window_ppi`] for window `w`; `None` for a window that does not float.
pub(crate) fn screen_ppi(app: &VectorcraftApp, w: Win) -> Option<crate::ScreenPpi> {
    let (u, n) = key(app, w)?;
    window_ppi(app, app.ui.floating.iter().find(|x| x.uid == u && x.view == n)?)
}

/// A floating window ([`views::key`]) with the density of the display that shows it
/// ([`window_ppi`]).
pub(crate) type FloatingPpi = ((u64, u32), Option<crate::ScreenPpi>);

/// Each floating window with the density of the display that shows it.
pub(crate) fn floating_ppi(app: &VectorcraftApp) -> Vec<FloatingPpi> {
    app.ui.floating.iter().map(|x| ((x.uid, x.view), window_ppi(app, x))).collect()
}

/// Forgets the densities found for the floating windows' displays.
pub(crate) fn forget_ppi(app: &mut VectorcraftApp) {
    for x in &mut app.ui.floating {
        x.ppi = None;
    }
}

/// Sets the density of the display that shows floating window `key`'s ([`views::key`]) OS window.
#[cfg(test)]
pub(crate) fn set_window_ppi(app: &mut VectorcraftApp, (uid, view): (u64, u32), ppi: Option<f64>) {
    if let Some(x) = app.ui.floating.iter_mut().find(|x| x.uid == uid && x.view == view) {
        x.ppi = Some(crate::ScreenPpi::Read(ppi));
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
    /// Counts the windows opened ([`DocWindow::number`]).
    opened: u32,
    /// The open documents (their uids) when [`opened`] last ran, or when the app started.
    known: Vec<u64>,
    /// The documents' views beyond the one each shows ([`views`]).
    pub(crate) views: Views,
    /// The panes drawn outside Tile in this frame so far (the floating windows', the main window's while the current
    /// window floats), back to front, with the art area each takes drops in; and those of the last frame.
    pub(crate) noted: Vec<(Win, egui::Rect)>,
    pub(crate) last_noted: Vec<(Win, egui::Rect)>,
    /// The main window's density and the floating windows' ([`floating_ppi`]) when the views were
    /// last settled each frame ([`VectorcraftApp::settle_screen_ppi_if_changed`]).
    pub(crate) settled: Option<(crate::ScreenPpi, Vec<FloatingPpi>)>,
    /// Display Print Size at 100% Zoom was off when the views were last settled each frame.
    pub(crate) print_size_off: bool,
    /// The last density the host read for the main window's display; the main window's views
    /// count from it while its display has not answered.
    pub(crate) main_read: Option<crate::ScreenPpi>,
}

impl Windows {
    /// No windows yet, with `known` (uids) the documents open when the app starts: they keep their
    /// tabs, whatever Open Documents As Tabs says.
    pub(crate) fn new(known: Vec<u64>) -> Self {
        Self { known, ..Default::default() }
    }
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

/// The size of a document window in `area`.
fn window_size(area: egui::Rect) -> egui::Vec2 {
    (area.size() * 0.6).max(MIN_SIZE).min(area.size())
}

/// The places Cascade gives `n` windows in `area`: each the same size, stepped [`CASCADE_STEP`]
/// down and to the right, starting again at the corner when the next would leave the area.
fn cascade_windows(area: egui::Rect, n: usize) -> Vec<egui::Rect> {
    // PLAN: needs `craft-behavior` in crates/ui-egui/Cargo.toml (not in this set's files)
    let size = window_size(area);
    // The margin shrinks where the area has no room for it.
    let margin = CASCADE_MARGIN.min((area.size() - size).min_elem()).max(0.0);
    let inside = egui::Rect::from_min_max(area.min + egui::Vec2::splat(margin), area.max);
    craft::behavior::placement::cascade_rects(inside, size, n, CASCADE_STEP)
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
    let places = cascade_windows(area(app), first + wins.len());
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
    let places = cascade_windows(area(app), app.ui.floating.len());
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

/// Gives the main window the keyboard when a floating document's OS window has it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn focus_main_window(app: &VectorcraftApp, ctx: &egui::Context) {
    if WindowHost::of(ctx).viewports_available() && app.ui.floating.iter().any(|w| w.focused) {
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
    }
}

/// Draw the floating document windows, after the main window's canvas, then the drop catcher of the main window's
/// pane, which waited for the current window's body. Call once per frame from the main window. Art released over a
/// pane in the frame the pointer reached it drops there then.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    show_windows(app, ui);
    crate::canvas::panes::catch_waiting(app, ui);
    crate::canvas::panes::drop_released(app);
}

/// [`show`]'s windows.
fn show_windows(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
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
                ui.ctx().request_repaint();
            }
        }
    } else if let Some(shown) = app.canvas.windows.presented.take() {
        let wins: Vec<Win> = shown.into_iter().filter_map(|k| find(app, k)).collect();
        for w in wins {
            if let Some(v) = view_mut(app, w) {
                v.fitted = false;
                // The area it last showed its art in is the main window's: it fits its window's.
                v.rect = egui::Rect::NOTHING;
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
    app.canvas.windows.current = current;
    let ctx = ui.ctx().clone();
    let os_windows = WindowHost::of(&ctx).viewports_available();
    // An OS window's place is in desktop points: the main window's own origin shifts it.
    let origin = if os_windows {
        ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).filter(|p| p.is_finite()).unwrap_or(egui::Pos2::ZERO).to_vec2()
    } else {
        egui::Vec2::ZERO
    };
    let mut close = None;
    let mut activate = None;
    for w in app.ui.floating.clone() {
        let Some(win) = find(app, (w.uid, w.view)) else { continue };
        let title = crate::chrome::document_window_title(app, win).unwrap_or_default();
        let is_current = views::current(app) == Some(win);
        let live = is_current && !crate::menus::home_showing(app);
        let mut pressed = false;
        let mut found = (None, None, false);
        let window = DocumentWindow::new(title).id_salt((w.uid, w.number, w.generation)).first_placed(w.place.translate(origin));
        let window = if raise == Some((w.uid, w.view)) { window.raise() } else { window };
        let out = window.show(ui, |ui, _| {
            // A press anywhere on the window, its title bar and tab included, makes its document current.
            pressed = pressed_on_layer(ui);
            crate::chrome::window_tab(app, ui, win);
            let areas = [(win, ui.available_rect_before_wrap())];
            let unfitted = crate::canvas::panes::refit_resized(app, &areas);
            // The display that shows the window, the size of its body and whether its view fits it in this frame.
            let ppi = if os_windows { crate::services::screen_ppi_at(&app.services.screens, ui.ctx()) } else { None };
            found = (ppi, Some(areas[0].1.size()), unfitted.first().copied().unwrap_or(false));
            if live {
                // The canvas fills the area below the tab, as a pane's does.
                let mut body = ui.new_child(egui::UiBuilder::new().max_rect(areas[0].1));
                show_document(app, &mut body, win, BodyKind::Live);
            } else {
                pressed |= crate::canvas::pane(app, ui, win, areas[0].1);
            }
            crate::canvas::panes::note_fits(app, &areas, &unfitted);
        });
        take_window_ppi(app, &ctx, ((w.uid, w.view), os_windows), win, found);
        if let Some(x) = app.ui.floating.iter_mut().find(|x| x.uid == w.uid && x.view == w.view) {
            x.focused = out.focused;
            // A move or resize of the fallback layer is where the window opens from now on.
            if let Some(placed) = out.placed {
                x.place = placed.translate(-origin);
            }
        }
        if out.close_requested.is_some() {
            close = Some(win);
        } else if (out.focus_gained || pressed) && !is_current && !app.any_dialog_open() {
            activate = Some(win);
        }
    }
    if let Some(win) = close {
        // The save prompt opens in the main window.
        if let Err(e) = close_window(app, win) {
            app.status(e);
        }
        if os_windows && app.any_dialog_open() {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
    } else if let Some(win) = activate {
        show_window(app, win);
        app.canvas.windows.current = views::current(app).and_then(|w| key(app, w));
    }
}

/// Takes `ppi`, the density of the display that floating window `key`'s ([`views::key`]) window
/// `win` was found on (`None` when it was found on none), whether it is an OS window of its own
/// (`os`), the size of its body and whether its view fitted the body in this frame, and settles
/// the window's view when the density changed ([`VectorcraftApp::settle_window_ppi`]). A view that
/// still has the zoom its window fitted it to at the window's present size keeps that zoom at its
/// first reading, and keeps fitting the window.
fn take_window_ppi(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    ((uid, number), os): ((u64, u32), bool),
    win: Win,
    (ppi, size, fitted_now): (Option<crate::ScreenPpi>, Option<egui::Vec2>, bool),
) {
    let Some(x) = app.ui.floating.iter_mut().find(|x| x.uid == uid && x.view == number) else { return };
    x.os = os;
    // Drawn inside the main window, it counts the main window's density.
    if !os {
        if x.ppi.take().is_some() {
            app.settle_window_ppi(win, true);
        }
        return;
    }
    // Off every display the host listed (moved past their edges, or its display went), it is
    // unread: its view counts from the density it was shown at until a display is found again.
    if ppi.is_none() {
        if x.ppi.take().is_some() {
            app.settle_window_ppi(win, true);
            ctx.request_repaint();
        }
        return;
    }
    if x.ppi == ppi {
        return;
    }
    // Its first reading, or a fit made this frame, is a baseline; a move from a display read
    // before keeps the zoom percentage.
    let first = !x.was_read;
    x.ppi = ppi;
    x.was_read |= matches!(ppi, Some(crate::ScreenPpi::Read(_)));
    let following = crate::canvas::panes::follows_fit(app, win, size);
    // A zoom set as a percentage (View › Zoom, Actual Size) is not a fit, and the first reading
    // keeps its percentage.
    let fit = following && view(app, win).is_some_and(|v| v.fit_zoom == Some(v.zoom));
    app.settle_window_ppi(win, !(fit && (first || fitted_now)));
    // A view that still fits its window keeps fitting it at the zoom the new density gave it.
    if following {
        crate::canvas::panes::refollow_fit(app, win);
    }
    ctx.request_repaint();
}

/// Whether the primary button went down over `ui`'s layer in this pass.
fn pressed_on_layer(ui: &egui::Ui) -> bool {
    let at = ui.ctx().input(|i| i.pointer.primary_pressed().then_some(i.pointer.interact_pos()).flatten());
    at.is_some_and(|at| ui.ctx().layer_id_at(at) == Some(ui.layer_id()))
}

#[cfg(test)]
mod tests;
