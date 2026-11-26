use craft::behavior::{CanvasView, ZoomScheme};
use craft::ui::canvas::{NavigationOptions, ViewCommands, ViewRequest, WheelScheme};
use serde_json::Value;
use vectorcraft_engine::Prefs;

use super::{VectorcraftApp, View, ViewInfo, panels};
use crate::state::ViewMove;

/// The canvas's navigation options for `p`: the wheel, Animated Zoom, the trackpad and touch gestures.
pub(crate) fn navigation(p: &Prefs) -> NavigationOptions {
    let mut options = NavigationOptions::default();
    options.wheel = if p.zoom_with_mouse_wheel { WheelScheme::Zoom } else { WheelScheme::Scroll };
    options.animated_zoom = crate::canvas::animated_zoom(p);
    options.scrubby_zoom = crate::canvas::animated_zoom(p);
    options.trackpad_rotate = p.trackpad_rotate_view;
    options.touch_gestures = p.touch_gestures;
    options.overscroll = true;
    options.flick_panning = false;
    options
}

/// The zoom rungs and range of the app, with 100 % at the device zoom that shows a document inch in
/// `print` screen points ([`crate::state::print_ppi`]; one document point per screen point when
/// `None`) on a display of `native_ppp` pixels per point.
pub(crate) fn zoom_scheme(print: Option<f64>, native_ppp: f32) -> ZoomScheme {
    ZoomScheme::ILLUSTRATOR.with_hundred(crate::state::zoom_baseline(print) * f64::from(native_ppp))
}

/// Whether view command `command` fits the view to the window (`Some(true)`), sets a zoom
/// percentage (`Some(false)`), or leaves the zoom alone (`None`).
pub(crate) fn fits_window(command: ViewCommands) -> Option<bool> {
    match command {
        ViewCommands::FitOnScreen | ViewCommands::FillScreen | ViewCommands::FitArtboard | ViewCommands::FitLayers => Some(true),
        ViewCommands::RotateClockwise | ViewCommands::RotateCounterclockwise | ViewCommands::ResetRotation => None,
        _ => Some(false),
    }
}

impl VectorcraftApp {
    /// Keep `views` aligned with the session's documents (a new one starts at its saved view, and
    /// opens in a floating window while Open Documents As Tabs is cleared). The views Window › New
    /// Window opened go with their documents.
    ///
    /// A view stays with its document when documents change order or close (`document.move`,
    /// `file.close`, also several of them in one recording or batch, #5663): the views are matched
    /// to the documents by the uid each was synced with, a closed document's view is dropped and a
    /// new document gets its saved view.
    pub fn sync_views(&mut self) {
        let docs = self.session.documents();
        if self.view_uids.len() == self.views.len() && docs.iter().map(|d| d.uid).ne(self.view_uids.iter().copied()) {
            let mut old: Vec<Option<View>> = std::mem::take(&mut self.views).into_iter().map(Some).collect();
            self.views = docs
                .iter()
                .map(|d| {
                    self.view_uids.iter().position(|u| *u == d.uid).and_then(|k| old.get_mut(k)).and_then(Option::take).unwrap_or_else(|| View::of(d))
                })
                .collect();
        }
        self.views.truncate(docs.len());
        self.views.extend(docs.iter().skip(self.views.len()).map(View::of));
        // A new view counts its zoom from the main window's display, where it is first shown.
        let main = self.main_ppi();
        for (view, doc) in self.views.iter_mut().zip(docs) {
            view.artboard = view.artboard.min(doc.doc.artboards.len().saturating_sub(1));
            view.shown_ppi.get_or_insert(main);
        }
        let uids: Vec<u64> = docs.iter().map(|d| d.uid).collect();
        if let Some(ctx) = &self.ui_ctx {
            for closed in self.view_uids.iter().filter(|u| !uids.contains(u)) {
                panels::layers::forget(ctx, *closed);
            }
            panels::layers::retain_open(ctx, &uids);
        }
        self.view_uids = uids;
        crate::doc_windows::retain_views(self);
        crate::doc_windows::opened(self);
        // Undo can remove the Artboard tool's active or additionally selected boards. Repair only
        // invalid indices: valid multiselection, drag state and tool options must survive history.
        if self.session.tool_id() != "artboard" || self.session.tool_busy() {
            return;
        }
        let Some(n) = self.session.active().map(|d| d.doc.artboards.len()).filter(|n| *n > 0) else { return };
        let Some(view) = self.session.active_index().and_then(|i| self.views.get_mut(i)) else { return };
        let options = self.session.tool_options();
        let Some(selected) = options["selected"].as_array() else { return };
        let mut valid: Vec<usize> = selected.iter().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).filter(|i| *i < n).collect();
        if valid.len() != selected.len() {
            if valid.is_empty() {
                valid.push(view.artboard);
            }
            if let Some(active) = valid.first() {
                view.artboard = *active;
            }
            self.session.set_tool_option("selected", &serde_json::json!(valid));
        }
    }

    /// The active document's uid, how many times the tool has started afresh
    /// ([`vectorcraft_engine::Session::tool_resets`]) and the artboards the Artboard tool has
    /// selected (null with another tool), which [`Self::follow_artboard_tool`] compares before and
    /// after a command.
    pub(super) fn artboard_tool_state(&self) -> (Option<u64>, u64, Value) {
        let selected = if self.session.tool_id() == "artboard" { self.session.tool_options()["selected"].clone() } else { Value::Null };
        (self.session.active().map(|d| d.uid), self.session.tool_resets(), selected)
    }

    /// After a command run with the Artboard tool, given [`Self::artboard_tool_state`] from before
    /// it: when the command chose another document, or started the tool afresh in the same one
    /// (File › Revert, a `command.batch` or `action.batch` that reverts it or switches to another
    /// document and back), the tool takes the document's active artboard, as choosing its tab
    /// does. When the command changed the artboards the tool has selected (Paste and Cut with the
    /// tool, `tool.setOption`), also after starting it afresh (a batch step after the revert or the
    /// switch), the tool's active artboard becomes the active one (the view's), as a click with the
    /// tool makes it.
    pub(super) fn follow_artboard_tool(&mut self, before: (Option<u64>, u64, Value)) {
        self.sync_views();
        if self.session.tool_id() != "artboard" {
            return;
        }
        let Some(n) = self.session.active().map(|d| d.doc.artboards.len()).filter(|n| *n > 0) else { return };
        let (doc, resets, selected) = self.artboard_tool_state();
        // The tool started afresh during the command, and no later step (a Paste or Cut) set its options.
        let reset = resets != before.1;
        let fresh = reset && !self.session.tool_set_since_reset();
        if fresh || (doc != before.0 && !reset) {
            panels::artboards::select(self, panels::artboards::selected(self, n));
        } else if (reset || selected != before.2)
            && let Some(i) = self.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok())
            && i < n
            && let Some(v) = self.view_mut()
        {
            v.artboard = i;
        }
    }

    pub fn view(&self) -> Option<&View> {
        self.session.active_index().and_then(|i| self.views.get(i))
    }

    /// Whether the active document is in Outline view (View › Outline); false without one.
    pub fn outline_view(&self) -> bool {
        self.view().is_some_and(|v| v.outline)
    }

    /// The zero point shown by rulers and position fields: the document's with global rulers, the
    /// active artboard's with artboard rulers. Command coordinates stay absolute.
    pub(crate) fn ruler_origin(&self) -> vectorcraft_geom::Point {
        let artboard = self.view().map_or(0, |v| v.artboard);
        self.session.active().map_or(vectorcraft_geom::Point::ZERO, |d| d.doc.ruler_origin_for(artboard))
    }

    /// The screen points an inch takes at 100% zoom with Display Print Size at 100% Zoom on, for
    /// the current window's display; `None` with it off ([`crate::state::print_ppi`]).
    pub(crate) fn print_ppi(&self) -> Option<f64> {
        self.print_ppi_of(crate::doc_windows::current(self))
    }

    /// [`Self::print_ppi`] for window `w`: the density its view's zoom counts from, which is the
    /// density of the display that shows it once [`Self::settle_screen_ppi`] has run.
    pub(crate) fn print_ppi_of(&self, w: Option<crate::doc_windows::Win>) -> Option<f64> {
        let counted = w.and_then(|w| crate::doc_windows::view(self, w)?.shown_ppi);
        let ppi = counted.unwrap_or_else(|| w.map_or(self.main_ppi(), |w| self.window_ppi(w)));
        crate::state::print_ppi(self.session.prefs.display_print_size, ppi.ppi())
    }

    /// The density of the display that shows window `w`: its own OS window's when it floats in one
    /// that the host found on a display, else the main window's ([`Self::main_ppi`]).
    fn window_ppi(&self, w: crate::doc_windows::Win) -> crate::ScreenPpi {
        crate::doc_windows::screen_ppi(self, w).unwrap_or(self.main_ppi())
    }

    /// The density the main window's views count from: the host's reading, or while the main
    /// window's display has not answered, the last reading, so every view there counts from the
    /// same density.
    fn main_ppi(&self) -> crate::ScreenPpi {
        match self.services.screen_ppi {
            crate::ScreenPpi::Unread => self.canvas.windows.main_read.unwrap_or(crate::ScreenPpi::Unread),
            read => read,
        }
    }

    /// Takes `ppi` as the density the host read for the main window's display, `None` for a
    /// display that reports no size ([`crate::Services::screen_ppi`]), and settles the views
    /// ([`Self::settle_screen_ppi`]). Returns whether the density changed.
    pub fn set_screen_ppi(&mut self, ppi: Option<f64>) -> bool {
        self.set_screen_reading(crate::ScreenPpi::Read(ppi))
    }

    /// [`Self::set_screen_ppi`] with the host's reading, [`crate::ScreenPpi::Unread`] while the
    /// main window's display is unknown or has not answered.
    pub fn set_screen_reading(&mut self, ppi: crate::ScreenPpi) -> bool {
        if self.services.screen_ppi == ppi {
            return false;
        }
        // A switch of the preference made since the last frame applies before the reading does.
        self.follow_print_size_switch();
        self.services.screen_ppi = ppi;
        if ppi != crate::ScreenPpi::Unread && self.session.prefs.display_print_size {
            self.canvas.windows.main_read = Some(ppi);
        }
        self.settle_screen_ppi();
        true
    }

    /// Gives each window's view the density of the display that shows it. With Display Print Size
    /// at 100% Zoom on, a view moved between two displays the host has read keeps its zoom
    /// percentage (a display that reports no size counts 96 points to an inch), so the art keeps
    /// its physical size on the new display; the views of other displays keep their zoom. At the
    /// first reading of a view's display, a view that still has the zoom a fit or the document's
    /// saved view gave it keeps that zoom, and any other view keeps its percentage, counted from
    /// 96 points to an inch until then.
    pub(crate) fn settle_screen_ppi(&mut self) {
        for w in crate::doc_windows::windows(self) {
            self.settle_window_ppi(w, true);
        }
    }

    /// [`Self::settle_screen_ppi`] for window `w` alone. With `keep` false (a fit made for the
    /// window), the view counts from the display's reading, without a change of zoom; while the
    /// display is being read, it counts from the last reading, and the reading still to come
    /// leaves its zoom alone.
    pub(crate) fn settle_window_ppi(&mut self, w: crate::doc_windows::Win, keep: bool) {
        let on = self.session.prefs.display_print_size;
        let now = self.window_ppi(w);
        let reading = crate::doc_windows::screen_ppi(self, w).unwrap_or(self.services.screen_ppi);
        let Some(v) = crate::doc_windows::view_mut(self, w) else { return };
        // Off, a view counts from no reading, so the first one after the preference is turned on
        // keeps the zoom chosen meanwhile.
        if !on {
            v.shown_ppi = Some(crate::ScreenPpi::Unread);
            v.fit_unread = false;
            return;
        }
        if !keep {
            // While the display is being read, the fit counts from the last reading, as the other
            // views there do, and the reading keeps its zoom. A floating window whose display has
            // not answered keeps counting from the density it was last shown at.
            match now {
                crate::ScreenPpi::Unread => {
                    v.shown_ppi.get_or_insert(now);
                }
                read => v.shown_ppi = Some(read),
            }
            v.fit_unread = reading == crate::ScreenPpi::Unread;
            return;
        }
        // A fit made while its display is being read waits for that reading.
        if v.fit_unread && reading == crate::ScreenPpi::Unread {
            return;
        }
        // A display that has not answered yet leaves the view counting from the last one read, so
        // the answer, when it comes, keeps the zoom percentage.
        let crate::ScreenPpi::Read(after) = now else {
            v.shown_ppi.get_or_insert(now);
            return;
        };
        let fitted = v.fit_zoom == Some(v.zoom);
        let before = match v.shown_ppi {
            _ if fitted && v.fit_unread => None,
            Some(crate::ScreenPpi::Read(before)) => Some(before),
            _ if fitted => None,
            _ => Some(None),
        };
        v.fit_unread = false;
        if let Some(before) = before
            && before != after
            && let (Some(b), Some(a)) = (crate::state::print_ppi(on, before), crate::state::print_ppi(on, after))
        {
            v.zoom = (v.zoom * a / b).clamp(0.0313, 640.0);
        }
        v.shown_ppi = Some(now);
    }

    /// [`Self::settle_screen_ppi`] when the main window's density or a floating window's changed
    /// since it last ran from here, or a window started or stopped floating, or Presentation Mode
    /// started or ended. With Display Print Size at 100% Zoom off, the desktop host does not read
    /// the displays: every density read before is forgotten, so the readings after the preference
    /// is turned on again count as first readings.
    pub(crate) fn settle_screen_ppi_if_changed(&mut self) {
        self.follow_print_size_switch();
        let key = (self.services.screen_ppi, crate::doc_windows::floating_ppi(self));
        if self.canvas.windows.settled.as_ref() != Some(&key) {
            self.settle_screen_ppi();
            self.canvas.windows.settled = Some(key);
        }
    }

    /// Takes in a change of Display Print Size at 100% Zoom since this last ran: turned off, every
    /// density read before is forgotten; turned on, every view's zoom counts as the one its first
    /// reading keeps.
    fn follow_print_size_switch(&mut self) {
        let off = !self.session.prefs.display_print_size;
        let was_off = std::mem::replace(&mut self.canvas.windows.print_size_off, off);
        if off && !was_off {
            crate::doc_windows::forget_ppi(self);
            self.services.screen_ppi = crate::ScreenPpi::Unread;
            self.canvas.windows.main_read = None;
            self.services.screens.clear();
            for w in crate::doc_windows::windows(self) {
                if let Some(v) = crate::doc_windows::view_mut(self, w) {
                    v.shown_ppi = Some(crate::ScreenPpi::Unread);
                }
            }
        }
        if was_off && !off {
            // The zoom chosen while it was off stays when the displays are read.
            for w in crate::doc_windows::windows(self) {
                if let Some(v) = crate::doc_windows::view_mut(self, w) {
                    v.fit_zoom = Some(v.zoom);
                }
            }
        }
    }

    pub fn view_mut(&mut self) -> Option<&mut View> {
        self.sync_views();
        let i = self.session.active_index()?;
        self.views.get_mut(i)
    }

    /// The active view as its canvas last showed it: none before the canvas has laid out with area,
    /// or without a document.
    pub fn canvas_view(&self) -> Option<CanvasView> {
        let v = self.view()?;
        (v.rect.width() > 0.0 && v.rect.height() > 0.0).then(|| v.canvas(self.ui_scaling(), v.rect))
    }

    /// Preferences › User Interface › UI Scaling, as the interface applies it.
    pub(crate) fn ui_scaling(&self) -> f32 {
        self.session.prefs.ui_scaling.clamp(0.75, 2.0) as f32
    }

    /// Ask the active view's canvas to run `request` in its next pass; `bounds` are the layers' bounds
    /// for Fit Layers.
    pub(crate) fn request_view(&mut self, request: ViewRequest, bounds: Option<vectorcraft_geom::Rect>) {
        if let Some(v) = self.view_mut() {
            v.request = Some((request, bounds));
        }
    }

    /// The active view was moved by a command since `before`: its canvas eases from `before`.
    pub(crate) fn ease_view(&mut self, before: View) {
        let scaling = self.ui_scaling();
        if let Some(v) = self.view_mut() {
            v.moved = Some(ViewMove::Ease(before.canvas(scaling, v.rect)));
        }
    }

    /// Runs `command` on the active view at once, in the canvas its last pass laid out, and eases the canvas there;
    /// `bounds` are the layers' bounds for Fit Layers. Without a window, or before the canvas was laid out, the canvas
    /// runs it in its next pass.
    pub(crate) fn view_now(&mut self, command: ViewCommands, bounds: Option<vectorcraft_geom::Rect>) {
        let (Some(ctx), Some(win), Some(_)) = (self.ui_ctx.clone(), crate::doc_windows::current(self), self.canvas_view()) else {
            return self.request_view(ViewRequest::Command(command), bounds);
        };
        let target = crate::canvas::view_target(self, win);
        let (scaling, ppp) = (self.ui_scaling(), ctx.pixels_per_point());
        self.before_view_command(win, command);
        let scheme = zoom_scheme(self.print_ppi_of(Some(win)), ppp / ctx.zoom_factor());
        let Some(v) = self.view_mut() else { return };
        let before = *v;
        let mut cv = v.canvas(scaling, v.rect);
        let layers = move || bounds;
        if command.apply(&mut cv, &target, Some(&layers), ppp, &scheme) {
            v.set_canvas(&cv, scaling);
            self.after_view_command(win, command);
            self.ease_view(before);
        }
    }

    /// Before view command `command` runs on window `w`'s view: a fit to the window is made for the
    /// display the window is on now, and a zoom percentage (Actual Size) counts from the density the
    /// view counts from, which a reading still to come rescales (Display Print Size at 100% Zoom).
    pub(crate) fn before_view_command(&mut self, w: crate::doc_windows::Win, command: ViewCommands) {
        if let Some(fit) = fits_window(command) {
            self.settle_window_ppi(w, !fit);
        }
    }

    /// After view command `command` ran on window `w`'s view: a fit to the window is kept at the
    /// first reading of its display; a zoom percentage keeps its percentage then.
    pub(crate) fn after_view_command(&mut self, w: crate::doc_windows::Win, command: ViewCommands) {
        if let Some(fit) = fits_window(command)
            && let Some(v) = crate::doc_windows::view_mut(self, w)
        {
            v.fit_zoom = fit.then_some(v.zoom);
        }
    }

    pub fn view_info(&self) -> ViewInfo {
        ViewInfo {
            // The tools measure their tolerances and widgets in the interface's points.
            zoom: self.view().map_or(1.0, |v| v.zoom / f64::from(self.ui_scaling())),
            outline: self.outline_view(),
            smart_guides: self.ui.view.smart_guides,
            guides: self.ui.view.guides_shown(),
            snap_to_grid: self.ui.view.snap_to_grid,
            pixel_preview: self.ui.view.pixel_preview,
            snap_to_pixel: self.ui.view.snap_to_pixel,
            // View › Hide Edges: the canvas draws neither the bounding box (with the type widget)
            // nor the Live Corners widgets, and the tools do not answer the pointer on them.
            show_bbox: self.ui.view.bounding_box && self.ui.view.edges,
            snap_to_point: self.ui.view.snap_to_point,
            snap_to_glyph: self.ui.view.snap_to_glyph,
            corner_widgets: self.ui.view.corner_widgets && self.ui.view.edges,
            shape_widgets: self.ui.view.shape_widgets,
            screen: self.screen_frame(),
            artboard: self.view().map_or(0, |v| v.artboard),
            precise: self.precise_cursors(),
        }
    }

    /// Whether the drawing tools show precise cursors: General › Use Precise Cursors is on, or Caps Lock is
    /// (`Session::caps_lock`, which the canvas keeps at the host's state and `view.capsLock` sets). The canvas's pointer
    /// and the tools' brush tips both follow it.
    pub(crate) fn precise_cursors(&self) -> bool {
        self.session.prefs.use_precise_cursors || self.session.caps_lock
    }

    /// Takes the host's Caps Lock state `on` when it changes: the session follows it until the next change, and a
    /// `view.capsLock` call between changes stays in place.
    pub(crate) fn follow_caps_lock(&mut self, on: bool) {
        if self.canvas.host_caps_lock == Some(on) {
            return;
        }
        self.canvas.host_caps_lock = Some(on);
        if self.session.caps_lock != on
            && let Err(err) = self.run("view.capsLock", serde_json::json!({ "on": on }))
        {
            log::error!("Could not update the Caps Lock state: {err}");
        }
    }
}

impl VectorcraftApp {
    /// The selection's bounding box, rotated with rotated objects ([`Session::transform_box`]); with
    /// anchors selected with Direct Selection, the box around them square to the page, which
    /// `object.setBounds` then edits (#1824). The canvas and the transform fields read it every
    /// frame: it is measured once per revision.
    pub fn selection_box(&mut self) -> Option<vectorcraft_doc::OrientedBox> {
        let st = self.session.active()?;
        let measure = self.session.bounds_measure();
        let key = (st.uid, st.revision, measure);
        if let Some((k, b)) = self.canvas.selection_box
            && k == key
        {
            return b;
        }
        let b = match vectorcraft_effects::preview::anchor_bounds_with(&st.selection, &st.doc, measure) {
            Some(r) => Some(vectorcraft_doc::OrientedBox::aligned(r)),
            None => self.session.transform_box(&st.selection.objects),
        };
        self.canvas.selection_box = Some((key, b));
        b
    }

    /// The selection's visual bounds (strokes included, effects left out), square to the page:
    /// measured once per revision, as the canvas reads it every frame (a traced photo selects a
    /// group of hundreds of thousands of paths).
    pub fn selection_bounds(&mut self) -> Option<vectorcraft_geom::Rect> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision);
        if let Some((k, b)) = self.canvas.selection_bounds
            && k == key
        {
            return b;
        }
        let b = st.doc.bounds_of(&st.selection.objects, true);
        self.canvas.selection_bounds = Some((key, b));
        b
    }

    /// The art and the artboards together, the span of the canvas's scroll bars: measured once
    /// per revision, as the canvas reads it every frame.
    pub fn content_bounds(&mut self) -> Option<vectorcraft_geom::Rect> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision);
        if let Some((k, b)) = self.canvas.content_bounds
            && k == key
        {
            return b;
        }
        let boards = st.doc.artboards.iter().map(|a| a.rect).reduce(|r, a| r.union(a));
        let b = vectorcraft_geom::union_opt(st.doc.art_bounds(), boards);
        self.canvas.content_bounds = Some((key, b));
        b
    }
}
