//! Window › Arrange › Tile: every open document in a pane of its own, edge to edge in the document
//! area. The current document's pane is the canvas ([`super::show`]). Each other pane draws its
//! document from a render of its own, with its artboards, guides and selected objects, and a press
//! in it makes its document the current one.

use std::collections::HashMap;
use std::sync::Arc;

use super::grid::LineLook;
use super::outline::{c32, node_outline, stroke_path, too_many_anchors};
use super::view::fit_zoom;
use super::{
    Affine, CacheKey, Point, Rect, RulerScale, Sense, Stroke, StrokeKind, Tokens, Ui, VectorcraftApp, View, Xf, artboard_edges, cache_key,
    frame_options, frame_view, grid, guides_of, json, paint_paper, pixel_region, pos2, rulers, shows_artboards, textured_quad, vec2, view_rects,
};
use crate::thumb_worker::ThumbWorker;

/// A pane's render: what it shows, the document, its size in pixels and view, and its options.
type PaneFrame = (CacheKey, Arc<vectorcraft_doc::Document>, ((u32, u32), Affine), vectorcraft_render::RenderOptions);

/// The renders that the panes of the documents other than the current one draw.
#[derive(Default)]
pub struct Panes {
    /// Renders off the UI thread (None: on it, as on wasm).
    worker: Option<ThumbWorker<usize, CacheKey>>,
    worker_started: bool,
    /// Renders on the UI thread when there is no worker.
    renderer: Option<vectorcraft_render::Renderer>,
    /// By document uid: the render shown and what it shows.
    shown: HashMap<usize, (CacheKey, egui::TextureHandle)>,
    /// By document uid: the render the worker is making.
    pending: HashMap<usize, CacheKey>,
    /// The panes the last frame drew, as (document index, pane rect).
    pub(crate) last: Vec<(usize, egui::Rect)>,
    /// By document uid: the size of the view's area its document was last fitted to in a pane,
    /// and the zoom and center the fit gave.
    fits: HashMap<usize, (egui::Vec2, f64, Point)>,
}

impl Panes {
    /// Forget every render and pane.
    fn clear(&mut self) {
        self.shown.clear();
        self.pending.clear();
        self.last.clear();
        self.fits.clear();
    }

    /// Upload `img`, the render `key` describes.
    fn upload(&mut self, ctx: &egui::Context, key: CacheKey, img: &vectorcraft_render::Rendered) {
        let color = super::color_image(img);
        // Pixel Preview's document pixels show hard-edged, as on the canvas.
        let options = if key.pixel.is_some() { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
        match self.shown.get_mut(&key.doc) {
            Some((k, tex)) => {
                tex.set(color, options);
                *k = key;
            }
            None => {
                let tex = ctx.load_texture(format!("arrange-pane-{}", key.doc), color, options);
                self.shown.insert(key.doc, (key, tex));
            }
        }
    }

    /// Bring the render of the document `key` describes up to `key` (`size` pixels through `view`),
    /// on the worker when there is one, then draw the latest render of that document through `xf`,
    /// turned and scaled into the view it was rendered for.
    fn draw(&mut self, ctx: &egui::Context, painter: &egui::Painter, xf: &Xf, frame: PaneFrame) {
        let (key, doc, ((w, h), view), opts) = frame;
        if !self.worker_started {
            self.worker_started = true;
            self.worker = ThumbWorker::spawn(ctx, "arrange-panes");
        }
        match self.worker.as_ref().map(ThumbWorker::poll) {
            Some(Some(done)) => {
                for d in done {
                    if self.pending.get(&d.key.doc) == Some(&d.key) {
                        self.pending.remove(&d.key.doc);
                    }
                    if let Some(img) = d.img {
                        self.upload(ctx, d.key, &img);
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
        let uid = key.doc;
        let current = self.shown.get(&uid).is_some_and(|(k, _)| *k == key) || self.pending.get(&uid) == Some(&key);
        if !current {
            let mut queued = false;
            if let Some(worker) = &self.worker {
                let (job_doc, job_opts) = (doc.clone(), opts.clone());
                queued = worker.submit(uid, key.clone(), Box::new(move |r| Some(r.render(&job_doc, w, h, view, &job_opts))));
                if queued {
                    self.pending.insert(uid, key.clone());
                } else {
                    self.worker = None;
                }
            }
            if !queued {
                let img = self.renderer.get_or_insert_with(vectorcraft_render::Renderer::new).render(&doc, w, h, view, &opts);
                self.upload(ctx, key, &img);
            }
        }
        if let Some((k, tex)) = self.shown.get(&uid) {
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

/// `window.arrange.tile`: every open document in a pane of its own, each fitted to its pane.
pub(crate) fn tile(app: &mut VectorcraftApp) -> Result<serde_json::Value, String> {
    app.sync_views();
    let n = app.session.documents().len();
    app.ui.tiled = n > 0;
    if n > 0 {
        app.ui.home = None;
    }
    for v in &mut app.views {
        v.fitted = false;
    }
    Ok(json!({"tiled": app.ui.tiled, "panes": n}))
}

/// `window.arrange.consolidateAllWindows`: one document at a time again, under its tab.
pub(crate) fn consolidate(app: &mut VectorcraftApp) -> Result<serde_json::Value, String> {
    app.ui.tiled = false;
    app.canvas.panes.clear();
    Ok(json!({"tiled": false}))
}

/// Draw the document area as panes while Tile is on and more than one document is open, outside
/// the Home screen and Presentation Mode. False when it did not: the caller draws the canvas.
pub(crate) fn tiled_area(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    let n = app.session.documents().len();
    if n == 0 {
        app.ui.tiled = false;
    }
    let tiling = app.ui.tiled && n > 1 && app.ui.screen_mode < 3 && !crate::menus::home_showing(app);
    let Some(active) = app.session.active_index().filter(|_| tiling) else {
        if !app.canvas.panes.shown.is_empty() || !app.canvas.panes.last.is_empty() {
            app.canvas.panes.clear();
        }
        return false;
    };
    app.sync_views();
    let live: Vec<usize> = app.session.documents().iter().map(|d| d.uid as usize).collect();
    app.canvas.panes.shown.retain(|uid, _| live.contains(uid));
    app.canvas.panes.pending.retain(|uid, _| live.contains(uid));
    app.canvas.panes.fits.retain(|uid, _| live.contains(uid));
    // A document opened or closed changes the grid, and so does coming back to the panes from one
    // document: each document fits its new pane.
    if app.canvas.panes.last.len() != n {
        for v in &mut app.views {
            v.fitted = false;
        }
    }
    let t = Tokens::get(ui.ctx());
    let ppp = ui.ctx().pixels_per_point();
    let cells = cells(n, ui.available_rect_before_wrap());
    let areas: Vec<egui::Rect> = cells.iter().map(|c| view_rects(app, *c, ppp).1).collect();
    let unfitted = refit_resized(app, &areas);
    let mut activate = None;
    // The other panes first, so that what the canvas draws past its pane (the Contextual Task Bar)
    // stays over them.
    for (i, cell) in cells.iter().copied().enumerate().filter(|(i, _)| *i != active) {
        if pane(app, ui, i, cell) {
            activate = Some(i);
        }
        ui.painter().rect_stroke(cell, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    }
    if let Some(cell) = cells.get(active).copied() {
        let mut canvas = ui.new_child(egui::UiBuilder::new().max_rect(cell));
        super::show(app, &mut canvas);
        ui.painter().rect_stroke(cell, 0.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
    }
    // Note the fits this frame made, for `refit_resized`.
    for ((i, area), was_unfitted) in areas.iter().enumerate().zip(unfitted) {
        if let (true, Some(v), Some(d)) = (was_unfitted, app.views.get(i).filter(|v| v.fitted), app.session.documents().get(i)) {
            app.canvas.panes.fits.insert(d.uid as usize, (area.size(), v.zoom, v.center));
        }
    }
    app.canvas.panes.last = cells.into_iter().enumerate().collect();
    if let Some(i) = activate {
        crate::chrome::show_document(app, i);
    }
    true
}

/// Unfit each document whose view is still where a pane fit left it when its view's area (in
/// `areas`, by document index) has changed size, as when the window or a panel is resized: it
/// fits its pane again. A view zoomed or scrolled since keeps its place. → which views are
/// unfitted now.
fn refit_resized(app: &mut VectorcraftApp, areas: &[egui::Rect]) -> Vec<bool> {
    let fits = &app.canvas.panes.fits;
    let docs = app.session.documents();
    for ((v, d), area) in app.views.iter_mut().zip(docs).zip(areas) {
        if let Some((size, zoom, center)) = fits.get(&(d.uid as usize))
            && *size != area.size()
            && *zoom == v.zoom
            && *center == v.center
        {
            v.fitted = false;
        }
    }
    app.views.iter().map(|v| !v.fitted).collect()
}

/// Fit `v` to show its artboard (the first if it has gone) in `rect`, as Fit Artboard in Window
/// does.
fn fit_pane(v: &mut View, doc: &vectorcraft_doc::Document, rect: egui::Rect) {
    if let Some(target) = doc.artboards.get(v.artboard).or(doc.artboards.first()).map(|a| a.rect) {
        v.zoom = fit_zoom(rect, target, v.rotation);
        v.center = target.center();
        v.fitted = true;
    }
}

/// The pane of document `i`, not the current one, in `cell`: its paper, art, grid, artboards,
/// guides and selected objects, and its rulers and the room of the canvas's scroll bars, so the
/// document stays where it is when the pane becomes the canvas. True when a press in the pane asks
/// for its document to become the current one.
fn pane(app: &mut VectorcraftApp, ui: &Ui, i: usize, cell: egui::Rect) -> bool {
    let t = Tokens::get(ui.ctx());
    let ppp = ui.ctx().pixels_per_point();
    let (full, rect) = view_rects(app, cell, ppp);
    let Some(st) = app.session.documents().get(i) else { return false };
    if let Some(v) = app.views.get_mut(i).filter(|v| !v.fitted) {
        fit_pane(v, &st.doc, rect);
    }
    let v = app.views.get(i).copied().unwrap_or_default();
    let xf = Xf::new(rect, &v);
    let painter = ui.painter_at(rect);
    paint_paper(app, st, ui, &painter, &xf, rect, &t);
    let grid_look = app.ui.view.grid.then(|| LineLook::grid(&app.session.prefs));
    let grids_in_back = app.session.prefs.grids_in_back;
    if let Some(look) = grid_look.filter(|_| grids_in_back) {
        grid(&painter, &xf, st.doc.grid.spacing, st.doc.grid.subdivisions, look);
    }
    let size = |l: f32| ((l * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX));
    let (w, h) = (size(rect.width()), size(rect.height()));
    // View › Pixel Preview, as the canvas draws it.
    let pixel = (app.ui.view.pixel_preview && v.zoom * ppp as f64 > 1.0).then(|| pixel_region(&xf)).flatten();
    let smooth_images = pixel.is_none() || app.session.prefs.anti_aliased_bitmaps;
    let mut key = cache_key(app, ui, st, &t, &v, (w, h, ppp), (pixel, smooth_images));
    // The current document's drag and Image Trace hold are not this document's.
    (key.dragging, key.trace_source) = (false, None);
    let opts = frame_options(app, &key, st.shown_mask(), smooth_images);
    let (fw, fh, view) = frame_view(pixel, (w, h), &v, ppp);
    let doc = st.doc.clone();
    app.canvas.panes.draw(ui.ctx(), &painter, &xf, (key, doc, ((fw, fh), view), opts));
    let Some(st) = app.session.documents().get(i) else { return false };
    if let Some(look) = grid_look.filter(|_| !grids_in_back) {
        grid(&painter, &xf, st.doc.grid.spacing, st.doc.grid.subdivisions, look);
    }
    if shows_artboards(app) {
        artboard_edges(app, &st.doc, (&painter, &xf, &t), v.artboard, |_| false);
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
    let resp = ui.interact(cell, egui::Id::unique(("arrange-pane", st.uid)), Sense::click_and_drag());
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
