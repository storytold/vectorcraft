//! Window › Arrange › Tile: every open document in a pane of its own, edge to edge in the document
//! area. The current document's pane is the canvas ([`super::show`]). Each other pane draws its
//! document from a render of its own, with its artboards, guides and selected objects, and a press
//! in it makes its document the current one. Each view Window › New Window opened has a pane of
//! its own ([`crate::doc_windows`]).

use std::collections::HashMap;
use std::sync::Arc;

use super::grid::LineLook;
use super::outline::{c32, node_outline, stroke_path, too_many_anchors};
use super::view::fit_zoom;
use super::{
    Affine, CacheKey, Point, Rect, RulerScale, Sense, Stroke, StrokeKind, Tokens, Ui, VectorcraftApp, View, Xf, artboard_edges, cache_key,
    dim_on_canvas, frame_options, frame_view, grid, guides_of, json, paint_paper, pixel_region, placed_key, plugin_key, pos2, rulers,
    shows_artboards, textured_quad, vec2, view_rects,
};
use crate::doc_windows::Win;
use crate::thumb_worker::ThumbWorker;

/// A pane's render: what it shows, the document, its size in pixels and view, and its options.
type PaneFrame = (CacheKey, Arc<vectorcraft_doc::Document>, ((u32, u32), Affine), vectorcraft_render::RenderOptions);

/// Whose render a pane draws: its document's uid and its view's number.
type Slot = (u64, u32);

/// The renders that the panes of the windows other than the current one draw.
#[derive(Default)]
pub struct Panes {
    /// Renders off the UI thread (None: on it, as on wasm).
    worker: Option<ThumbWorker<Slot, (Slot, CacheKey)>>,
    worker_started: bool,
    /// Renders on the UI thread when there is no worker.
    renderer: Option<vectorcraft_render::Renderer>,
    /// By slot: the render shown and what it shows.
    pub(crate) shown: HashMap<Slot, (CacheKey, egui::TextureHandle)>,
    /// By slot: the render the worker is making.
    pending: HashMap<Slot, CacheKey>,
    /// The panes the last frame drew, as (window, pane rect).
    pub(crate) last: Vec<(Win, egui::Rect)>,
    /// By slot: the size of the view's area its view was last fitted to in a pane, and the zoom and
    /// center the fit gave.
    fits: HashMap<Slot, (egui::Vec2, f64, Point)>,
}

impl Panes {
    /// Forget every render and pane.
    fn clear(&mut self) {
        self.shown.clear();
        self.pending.clear();
        self.last.clear();
        self.fits.clear();
    }

    /// Upload `img`, the render `key` describes, for `slot`.
    fn upload(&mut self, ctx: &egui::Context, slot: Slot, key: CacheKey, img: &vectorcraft_render::Rendered) {
        let color = super::color_image(img);
        // Pixel Preview's document pixels show hard-edged, as on the canvas.
        let options = if key.pixel.is_some() { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
        match self.shown.get_mut(&slot) {
            Some((k, tex)) => {
                tex.set(color, options);
                *k = key;
            }
            None => {
                let tex = ctx.load_texture(format!("arrange-pane-{}-{}", slot.0, slot.1), color, options);
                self.shown.insert(slot, (key, tex));
            }
        }
    }

    /// Bring `slot`'s render up to `key` (`size` pixels through `view`), on the worker when there
    /// is one, then draw the slot's latest render through `xf`, turned and scaled into the view it
    /// was rendered for.
    fn draw(&mut self, ctx: &egui::Context, painter: &egui::Painter, xf: &Xf, slot: Slot, frame: PaneFrame) {
        let (key, doc, ((w, h), view), opts) = frame;
        if !self.worker_started {
            self.worker_started = true;
            self.worker = ThumbWorker::spawn(ctx, "arrange-panes");
        }
        match self.worker.as_ref().map(ThumbWorker::poll) {
            Some(Some(done)) => {
                for d in done {
                    let (s, k) = d.key;
                    if self.pending.get(&s) == Some(&k) {
                        self.pending.remove(&s);
                    }
                    if let Some(img) = d.img {
                        self.upload(ctx, s, k, &img);
                    }
                }
            }
            // The thread is gone: render here from now on.
            Some(None) => {
                self.worker = None;
                self.pending.clear();
            }
            None => {}
        }
        let current = self.shown.get(&slot).is_some_and(|(k, _)| *k == key) || self.pending.get(&slot) == Some(&key);
        if !current {
            let mut queued = false;
            if let Some(worker) = &self.worker {
                let (job_doc, job_opts) = (doc.clone(), opts.clone());
                queued = worker.submit(slot, (slot, key.clone()), Box::new(move |r| Some(r.render(&job_doc, w, h, view, &job_opts))));
                if queued {
                    self.pending.insert(slot, key.clone());
                } else {
                    self.worker = None;
                }
            }
            if !queued {
                let img = self.renderer.get_or_insert_with(vectorcraft_render::Renderer::new).render(&doc, w, h, view, &opts);
                self.upload(ctx, slot, key, &img);
            }
        }
        if let Some((k, tex)) = self.shown.get(&slot) {
            // Document pixels: placed over the document rect they cover.
            if let Some([x0, y0, x1, y1]) = k.pixel {
                textured_quad(painter, tex.id(), &xf.quad(Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64)), (1.0, 1.0));
                return;
            }
            let ppp = if k.ppp > 0.0 { k.ppp } else { 1.0 };
            let old_rect = egui::Rect::from_center_size(xf.rect.center(), vec2(k.w as f32 / ppp, k.h as f32 / ppp));
            let old = Xf { rect: old_rect, zoom: k.zoom, center: Point::new(k.cx, k.cy), rot: k.rot.to_radians() };
            let q = [old_rect.left_top(), old_rect.right_top(), old_rect.right_bottom(), old_rect.left_bottom()].map(|c| xf.to_screen(old.to_doc(c)));
            textured_quad(painter, tex.id(), &q, (1.0, 1.0));
        }
    }
}

/// The grid of `n` panes in `area`, filled row by row: as many columns as the square root of `n`
/// rounded up, and as many rows as those columns need. The panes meet edge to edge and cover
/// `area`; the spare cells of the last row stay empty.
pub(crate) fn cells(n: usize, area: egui::Rect) -> Vec<egui::Rect> {
    if n == 0 {
        return vec![];
    }
    let mut cols = 1;
    while cols * cols < n {
        cols += 1;
    }
    let rows = n.div_ceil(cols);
    let x = |c: usize| if c == cols { area.right() } else { area.left() + area.width() * c as f32 / cols as f32 };
    let y = |r: usize| if r == rows { area.bottom() } else { area.top() + area.height() * r as f32 / rows as f32 };
    (0..n).map(|i| egui::Rect::from_min_max(pos2(x(i % cols), y(i / cols)), pos2(x(i % cols + 1), y(i / cols + 1)))).collect()
}

/// `window.arrange.tile`: every open document, and each further view of one, in a pane of its own,
/// each fitted to its pane. With fewer than two windows open (when the menu item is grayed out) it
/// leaves the view and `tiled` as they are. The windows floating come back under the tabs for it.
pub(crate) fn tile(app: &mut VectorcraftApp) -> Result<serde_json::Value, String> {
    app.sync_views();
    let n = crate::doc_windows::windows(app).len();
    if n < 2 {
        return Ok(json!({"tiled": app.ui.tiled, "panes": n}));
    }
    crate::doc_windows::dock_all(app);
    app.ui.tiled = true;
    app.ui.home = None;
    crate::doc_windows::unfit_all(app);
    Ok(json!({"tiled": app.ui.tiled, "panes": n}))
}

/// `window.arrange.consolidateAllWindows`: one document at a time again, under its tab, the
/// documents in floating windows included.
pub(crate) fn consolidate(app: &mut VectorcraftApp) -> Result<serde_json::Value, String> {
    app.ui.tiled = false;
    crate::doc_windows::dock_all(app);
    app.canvas.panes.clear();
    Ok(json!({"tiled": false, "floating": []}))
}

/// Draw the document area as panes while Tile is on and more than one window is under the tabs,
/// outside the Home screen and Presentation Mode. False when it did not: the caller draws the
/// canvas. The floating windows have no pane.
pub(crate) fn tiled_area(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    if app.session.documents().is_empty() {
        app.ui.tiled = false;
    }
    let docked = crate::doc_windows::docked_windows(app);
    let n = docked.len();
    let tiling = app.ui.tiled && n > 1 && app.ui.screen_mode < 3 && !crate::menus::home_showing(app);
    if !tiling {
        // The floating windows draw their panes from the same renders.
        if app.ui.floating.is_empty() && (!app.canvas.panes.shown.is_empty() || !app.canvas.panes.last.is_empty()) {
            app.canvas.panes.clear();
        }
        app.canvas.panes.last.clear();
        return false;
    }
    // The current window's pane is the canvas, unless it is a floating one.
    let active = crate::doc_windows::current(app).filter(|a| docked.contains(a));
    app.sync_views();
    let live: Vec<Slot> = crate::doc_windows::windows(app).into_iter().filter_map(|w| slot(app, w)).collect();
    app.canvas.panes.shown.retain(|s, _| live.contains(s));
    app.canvas.panes.pending.retain(|s, _| live.contains(s));
    app.canvas.panes.fits.retain(|s, _| live.contains(s));
    // A window opened, closed, floated or docked changes the grid, and so does coming back to the
    // panes from one window: each window fits its new pane.
    if app.canvas.panes.last.iter().map(|(w, _)| *w).ne(docked.iter().copied()) {
        for w in &docked {
            if let Some(v) = crate::doc_windows::view_mut(app, *w) {
                v.fitted = false;
            }
        }
    }
    let t = Tokens::get(ui.ctx());
    let ppp = ui.ctx().pixels_per_point();
    let cells: Vec<(Win, egui::Rect)> = docked.iter().copied().zip(cells(n, ui.available_rect_before_wrap())).collect();
    let areas: Vec<(Win, egui::Rect)> = cells.iter().map(|(w, cell)| (*w, view_rects(app, *cell, ppp).1)).collect();
    let unfitted = refit_resized(app, &areas);
    let mut activate = None;
    // The other panes first, so that what the canvas draws past its pane (the Contextual Task Bar)
    // stays over them.
    for (w, cell) in cells.iter().copied().filter(|(w, _)| Some(*w) != active) {
        if pane(app, ui, w, cell) {
            activate = Some(w);
        }
        ui.painter().rect_stroke(cell, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    }
    if let Some((_, cell)) = cells.iter().copied().find(|(w, _)| Some(*w) == active) {
        let mut canvas = ui.new_child(egui::UiBuilder::new().max_rect(cell));
        super::show(app, &mut canvas);
        ui.painter().rect_stroke(cell, 0.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
    }
    // Note the fits this frame made, for `refit_resized`.
    for ((w, area), was_unfitted) in areas.iter().zip(unfitted) {
        if let (true, Some(v), Some(s)) = (was_unfitted, crate::doc_windows::view(app, *w).filter(|v| v.fitted).copied(), slot(app, *w)) {
            app.canvas.panes.fits.insert(s, (area.size(), v.zoom, v.center));
        }
    }
    app.canvas.panes.last = cells;
    if let Some(w) = activate {
        crate::doc_windows::show_window(app, w);
    }
    true
}

/// Window `w`'s [`Slot`].
fn slot(app: &VectorcraftApp, (i, n): Win) -> Option<Slot> {
    app.session.documents().get(i).map(|d| (d.uid, n))
}

/// Unfit each window whose view is still where a pane fit left it when its view's area (`areas`,
/// by window) has changed size, as when the window or a panel is resized: it fits its pane again.
/// A view zoomed or scrolled since keeps its place. → which of `areas`' views are unfitted now.
fn refit_resized(app: &mut VectorcraftApp, areas: &[(Win, egui::Rect)]) -> Vec<bool> {
    areas
        .iter()
        .map(|(w, area)| {
            let fit = slot(app, *w).and_then(|s| app.canvas.panes.fits.get(&s).copied());
            let Some(v) = crate::doc_windows::view_mut(app, *w) else { return false };
            if let Some((size, zoom, center)) = fit
                && size != area.size()
                && zoom == v.zoom
                && center == v.center
            {
                v.fitted = false;
            }
            !v.fitted
        })
        .collect()
}

/// Fit `v` to show its artboard (the first if it has gone) in `rect`, with the interface zoomed by
/// `ui_zoom`, as Fit Artboard in Window does.
fn fit_pane(v: &mut View, doc: &vectorcraft_doc::Document, rect: egui::Rect, ui_zoom: f32) {
    if let Some(target) = doc.artboards.get(v.artboard).or(doc.artboards.first()).map(|a| a.rect) {
        v.zoom = fit_zoom(rect, target, v.rotation, ui_zoom);
        v.center = target.center();
        v.fitted = true;
    }
}

/// The pane of window `w` (document `i`'s view `n`), not the current one, in `cell`: its paper,
/// art, grid, artboards, guides and selected objects, and its rulers and the room of the canvas's
/// scroll bars, so the document stays where it is when the pane becomes the canvas. True when a
/// press in the pane asks for its window to become the current one. A floating window draws its
/// view with this too ([`crate::doc_windows`]).
pub(crate) fn pane(app: &mut VectorcraftApp, ui: &Ui, w: Win, cell: egui::Rect) -> bool {
    let t = Tokens::get(ui.ctx());
    let ppp = ui.ctx().pixels_per_point();
    let (full, rect) = view_rects(app, cell, ppp);
    let Some(st) = app.session.documents().get(w.0) else { return false };
    let doc = st.doc.clone();
    let ui_zoom = ui.ctx().zoom_factor();
    if let Some(v) = crate::doc_windows::view_mut(app, w).filter(|v| !v.fitted) {
        fit_pane(v, &doc, rect, ui_zoom);
    }
    let v = crate::doc_windows::view(app, w).copied().unwrap_or_default();
    let Some(st) = app.session.documents().get(w.0) else { return false };
    let slot = (st.uid, w.1);
    let xf = Xf::at(rect, &v, ui_zoom);
    let painter = ui.painter_at(rect);
    paint_paper(app, st, ui, &painter, &xf, rect, (&t, v.outline));
    let grid_look = app.ui.view.grid.then(|| LineLook::grid(&app.session.prefs));
    let grids_in_back = app.session.prefs.grids_in_back;
    if let Some(look) = grid_look.filter(|_| grids_in_back) {
        grid(&painter, &xf, st.doc.grid.spacing, st.doc.grid.subdivisions, look);
    }
    let size = |l: f32| ((l * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX));
    let (pw, ph) = (size(rect.width()), size(rect.height()));
    // View › Pixel Preview, as the canvas draws it.
    let pixel = (app.ui.view.pixel_preview && xf.zoom * ppp as f64 > 1.0).then(|| pixel_region(&xf)).flatten();
    let smooth_images = pixel.is_none() || app.session.prefs.anti_aliased_bitmaps;
    let plugins = plugin_key(&mut app.canvas.plugin_keys, &app.session, st);
    let placed = placed_key(&mut app.canvas.placed_keys, &app.session, st);
    let drawn = v.on_screen(ui_zoom);
    let mut key = cache_key(app, ui, st, &t, &drawn, (pw, ph, ppp), (pixel, smooth_images, plugins, placed));
    // The current document's drag and Image Trace hold are not this document's, and this view has
    // its own Outline or Preview.
    (key.dragging, key.trace_source, key.outline) = (false, None, v.outline);
    let opts = frame_options(app, &key, st.shown_mask(), smooth_images);
    let (fw, fh, view) = frame_view(pixel, (pw, ph), &drawn, ppp);
    let doc = st.doc.clone();
    app.canvas.panes.draw(ui.ctx(), &painter, &xf, slot, (key, doc, ((fw, fh), view), opts));
    let Some(st) = app.session.documents().get(w.0) else { return false };
    if let Some(look) = grid_look.filter(|_| !grids_in_back) {
        grid(&painter, &xf, st.doc.grid.spacing, st.doc.grid.subdivisions, look);
    }
    if shows_artboards(app) {
        artboard_edges(app, &st.doc, (&painter, &xf, &t), (v.artboard, dim_on_canvas(&t, v.outline)), |_| false);
    }
    if app.ui.view.guides {
        guides_of(app, st, (&painter, &xf, &t), rect);
    }
    if app.ui.view.edges {
        selected_objects(st, &painter, &xf);
    }
    if app.ui.view.rulers && app.ui.screen_mode < 3 {
        rulers(ui, full, &xf, None, RulerScale::document(app.session.general_unit(), st.doc.ruler_origin_for(v.artboard)), &t);
    }
    if app.ui.screen_mode < 2 {
        let bars = [
            egui::Rect::from_min_max(pos2(full.right(), cell.top()), cell.max),
            egui::Rect::from_min_max(pos2(cell.left(), full.bottom()), cell.max),
        ];
        for bar in bars {
            ui.painter().rect_filled(bar, 0.0, t.ruler);
        }
    }
    let resp = ui.interact(cell, egui::Id::unique(("arrange-pane", st.uid, w.1)), Sense::click_and_drag());
    resp.contains_pointer() && ui.input(|inp| inp.pointer.any_pressed())
}

/// The outlines of the objects selected in `st`, in their layers' colors; their bounds when they
/// have too many points to outline.
fn selected_objects(st: &vectorcraft_engine::DocState, painter: &egui::Painter, xf: &Xf) {
    let symbols = st.doc.symbol_boxes();
    let nodes: Vec<(vectorcraft_doc::NodeId, &vectorcraft_doc::Node)> =
        st.selection.objects.iter().filter_map(|id| Some((*id, st.doc.node(*id)?))).collect();
    let color = |id: vectorcraft_doc::NodeId| Stroke::new(1.0, c32(st.doc.layer_color(id)));
    if too_many_anchors(nodes.iter().map(|(_, n)| *n)) {
        let bounds = nodes.iter().filter_map(|(_, n)| n.geometric_bounds_with(&symbols)).reduce(|a, b| a.union(b));
        if let (Some(b), Some((id, _))) = (bounds, nodes.first()) {
            stroke_path(painter, &vectorcraft_geom::shapes::rectangle(b).to_bezpath(), xf, color(*id));
        }
        return;
    }
    for (id, n) in nodes {
        stroke_path(painter, &node_outline(n, &symbols), xf, color(id));
    }
}

#[cfg(test)]
mod tests;
