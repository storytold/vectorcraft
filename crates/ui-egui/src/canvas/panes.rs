//! Window › Arrange › Tile: every open document in a pane of its own, edge to edge in the document
//! area, in a grid, side by side or stacked; the 2-up to 6-up layouts show that many and leave the
//! rest under their tabs ([`Layout`]). The current document's pane is the canvas ([`super::show`]). Each other pane draws its
//! document from a render of its own, with its artboards, guides and selected objects, and a press
//! in it makes its document the current one. A Selection drag released in it drops the dragged
//! artwork into its document ([`crate::document_drag::drop_on_pane`]). Each view Window › New
//! Window opened has a pane of its own ([`crate::doc_windows`]).

use std::collections::HashMap;
use std::sync::Arc;

use super::grid::LineLook;
use super::outline::{c32, node_outline, to_screen_path, too_many_anchors};
use super::view::fit_zoom;
use super::{
    Affine, BezPath, CacheKey, Point, Rect, RulerScale, Sense, Stroke, StrokeKind, Tokens, Ui, VectorcraftApp, View, Xf, artboard_edges, backdrop,
    cache_key, dim_on_canvas, frame_options, frame_ppp, frame_view, grid, guides_of, json, paint_paper, pixel_region, placed_key, plugin_key, pos2,
    rulers, shows_artboards, textured_quad, vec2, view_rects,
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
    pub(crate) worker_started: bool,
    /// Renders on the UI thread when there is no worker.
    renderer: Option<vectorcraft_render::Renderer>,
    /// By slot: the render shown and what it shows.
    pub(crate) shown: HashMap<Slot, (CacheKey, egui::TextureHandle)>,
    /// By slot: the render the worker is making.
    pending: HashMap<Slot, CacheKey>,
    /// The panes the last frame drew, as (window, pane rect).
    pub(crate) last: Vec<(Win, egui::Rect)>,
    /// The slots of `last`'s windows, in the same order: which windows they were, as document
    /// indices shift when a document closes.
    last_slots: Vec<Option<Slot>>,
    /// By slot: the size of the view's area its view was last fitted to in a pane, and the zoom and
    /// center the fit gave.
    fits: HashMap<Slot, (egui::Vec2, f64, Point)>,
    /// By slot: the outlines of the selected objects the pane drew last, and what they show.
    outlines: HashMap<Slot, Outlines>,
    /// How many times a pane's outlines were made.
    outlines_made: u64,
}

/// A pane's selected-object outlines as screen shapes, and the document revision, document,
/// selection and screen mapping they were made for.
struct Outlines {
    revision: u64,
    /// A weak reference to the document: while it exists, no other document can take the
    /// allocation's address.
    doc: std::sync::Weak<vectorcraft_doc::Document>,
    selection: Vec<vectorcraft_doc::NodeId>,
    affine: [f64; 6],
    shapes: Vec<egui::Shape>,
}

impl Outlines {
    /// Whether these outlines show `st`'s selection through the screen mapping `affine`.
    fn shows(&self, st: &vectorcraft_engine::DocState, affine: &[f64; 6]) -> bool {
        self.revision == st.revision && self.doc.ptr_eq(&Arc::downgrade(&st.doc)) && self.affine == *affine && self.selection == st.selection.objects
    }
}

impl Panes {
    /// Forget every render and pane.
    fn clear(&mut self) {
        self.shown.clear();
        self.pending.clear();
        self.last.clear();
        self.last_slots.clear();
        self.fits.clear();
        self.outlines.clear();
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
    /// is one, converted into the display profile `display`, then draw the slot's latest render
    /// through `xf`, turned and scaled into the view it was rendered for.
    fn draw(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        xf: &Xf,
        slot: Slot,
        frame: PaneFrame,
        display: Option<Arc<vectorcraft_color::cms::display::DisplayTransform>>,
    ) {
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
            // The document draws through its own assigned profiles, not the front document's, on
            // the worker too.
            let cms = vectorcraft_engine::cmd::colormgmt::doc_cms(&doc);
            let mut queued = false;
            if let Some(worker) = &self.worker {
                let (job_doc, job_opts, job_display, job_cms, job_key) = (doc.clone(), opts.clone(), display.clone(), cms.clone(), key.clone());
                queued = worker.submit(
                    slot,
                    (slot, key.clone()),
                    Box::new(move |r| Some(render_pane(r, (&job_doc, w, h, view, &job_opts), job_display.as_deref(), job_cms, job_key.over()))),
                );
                if queued {
                    self.pending.insert(slot, key.clone());
                } else {
                    self.worker = None;
                }
            }
            if !queued {
                let r = self.renderer.get_or_insert_with(vectorcraft_render::Renderer::new);
                let img = render_pane(r, (&doc, w, h, view, &opts), display.as_deref(), cms, key.over());
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

/// Render `doc` (`w` × `h` pixels through `view`, with `opts`) through the color settings `cms`,
/// then convert it into the display profile `display`, blended with the backdrop `over` first
/// ([`crate::display_profile::convert_over`]).
fn render_pane(
    r: &mut vectorcraft_render::Renderer,
    (doc, w, h, view, opts): (&vectorcraft_doc::Document, u32, u32, Affine, &vectorcraft_render::RenderOptions),
    display: Option<&vectorcraft_color::cms::display::DisplayTransform>,
    cms: Arc<vectorcraft_color::cms::Cms>,
    over: Option<(&crate::display_profile::Backdrop, f64)>,
) -> vectorcraft_render::Rendered {
    let mut img = vectorcraft_color::cms::with_active(cms, || r.render(doc, w, h, view, opts));
    let width = img.width;
    crate::display_profile::convert_over(display, &mut img.pixels, width, view, over);
    img
}

/// How Tile lays out the panes (`window.arrange.tile`'s `layout`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// A grid: as many columns as the square root of the number of panes rounded up.
    #[default]
    Grid,
    /// Side by side in one row, each pane as tall as the document area.
    Vertical,
    /// Stacked in one column, each pane as wide as the document area.
    Horizontal,
    /// At most this many panes (2, 3, 4 or 6); the other windows stay under their tabs.
    Up(usize),
}

impl Layout {
    /// Each layout with its name in `layout`, in the order the menus list them.
    pub(crate) const ALL: [(&'static str, Layout); 7] = [
        ("grid", Layout::Grid),
        ("vertical", Layout::Vertical),
        ("horizontal", Layout::Horizontal),
        ("2up", Layout::Up(2)),
        ("3up", Layout::Up(3)),
        ("4up", Layout::Up(4)),
        ("6up", Layout::Up(6)),
    ];

    /// The layout's name in `layout`.
    pub(crate) fn name(self) -> &'static str {
        Self::ALL.iter().find(|(_, l)| *l == self).map_or("grid", |(n, _)| n)
    }

    /// The layout `p`'s `layout` names, the grid without one. Any other value is an error.
    pub(crate) fn from_params(p: &serde_json::Value) -> Result<Layout, String> {
        let key = "layout";
        match p.get(key) {
            None => Ok(Layout::Grid),
            Some(v) => v
                .as_str()
                .and_then(|s| Self::ALL.iter().find(|(n, _)| *n == s))
                .map(|(_, l)| *l)
                .ok_or_else(|| format!("`{key}` must be one of {}", Self::ALL.map(|(n, _)| n).join(", "))),
        }
    }

    /// How many of `n` windows the layout gives a pane.
    fn panes(self, n: usize) -> usize {
        match self {
            Layout::Up(k) => n.min(k),
            _ => n,
        }
    }

    /// How many columns the layout puts `n` panes in.
    fn columns(self, n: usize) -> usize {
        match self {
            Layout::Grid => grid_columns(n),
            Layout::Vertical => n,
            Layout::Horizontal => 1,
            Layout::Up(k) if k <= 3 => n,
            Layout::Up(k) => grid_columns(k).min(n),
        }
    }
}

/// The square root of `n` rounded up.
fn grid_columns(n: usize) -> usize {
    let mut cols = 1;
    while cols * cols < n {
        cols += 1;
    }
    cols
}

/// The `n` panes `layout` places in `area`, filled row by row, in as many rows as its columns
/// need (the grid: as many columns as the square root of `n` rounded up). The panes meet edge to
/// edge and cover `area`; the spare cells of the last row stay empty.
pub(crate) fn layout_cells(layout: Layout, n: usize, area: egui::Rect) -> Vec<egui::Rect> {
    if n == 0 {
        return vec![];
    }
    let cols = layout.columns(n).clamp(1, n);
    let rows = n.div_ceil(cols);
    let x = |c: usize| if c == cols { area.right() } else { area.left() + area.width() * c as f32 / cols as f32 };
    let y = |r: usize| if r == rows { area.bottom() } else { area.top() + area.height() * r as f32 / rows as f32 };
    (0..n).map(|i| egui::Rect::from_min_max(pos2(x(i % cols), y(i / cols)), pos2(x(i % cols + 1), y(i / cols + 1)))).collect()
}

/// The windows of `docked` (in tab order) that `layout` gives a pane: all of them, or as many of
/// the first ones as it shows, with the current window `active` in the last pane when it is not
/// among them.
pub(crate) fn shown_windows(layout: Layout, docked: &[Win], active: Option<Win>) -> Vec<Win> {
    let k = layout.panes(docked.len());
    let mut shown = docked[..k].to_vec();
    if let (Some(a), Some(last)) = (active.filter(|a| !shown.contains(a) && docked.contains(a)), shown.last_mut()) {
        *last = a;
    }
    shown
}

/// `window.arrange.tile`: every open document, and each further view of one, in a pane of its own
/// as `layout` places them (the grid without one), each fitted to its pane. With fewer than two
/// windows open (when the menu item is grayed out) it leaves the view, `tiled` and the layout as
/// they are. The windows floating come back under the tabs for it. A `layout` that is not one of
/// [`Layout::ALL`]'s names is an error.
pub(crate) fn tile(app: &mut VectorcraftApp, p: &serde_json::Value) -> Result<serde_json::Value, String> {
    let layout = Layout::from_params(p)?;
    app.sync_views();
    let n = crate::doc_windows::windows(app).len();
    if n < 2 {
        return Ok(json!({"tiled": app.ui.tiled, "panes": n, "layout": app.ui.tile_layout.name()}));
    }
    crate::doc_windows::dock_all(app);
    app.ui.tiled = true;
    app.ui.tile_layout = layout;
    app.ui.home = None;
    crate::doc_windows::unfit_all(app);
    Ok(json!({"tiled": app.ui.tiled, "panes": layout.panes(n), "layout": layout.name()}))
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
        // Only the floating windows draw panes now.
        let floating = &app.ui.floating;
        app.canvas.panes.outlines.retain(|(u, n), _| floating.iter().any(|w| w.uid == *u && w.view == *n));
        app.canvas.panes.last_slots.clear();
        return false;
    }
    // The current window's pane is the canvas, unless it is a floating one.
    let active = crate::doc_windows::current(app).filter(|a| docked.contains(a));
    // The 2-up to 6-up layouts leave the windows past their panes under the tabs.
    let layout = app.ui.tile_layout;
    let shown = shown_windows(layout, &docked, active);
    app.sync_views();
    // The renders of the panes and the floating windows stay; a window under its tab has none.
    let live: Vec<Slot> =
        crate::doc_windows::windows(app).into_iter().filter(|w| shown.contains(w) || !docked.contains(w)).filter_map(|w| slot(app, w)).collect();
    app.canvas.panes.shown.retain(|s, _| live.contains(s));
    app.canvas.panes.pending.retain(|s, _| live.contains(s));
    app.canvas.panes.fits.retain(|s, _| live.contains(s));
    app.canvas.panes.outlines.retain(|s, _| live.contains(s));
    // A window that moved to another pane fits it, and so does a window whose pane changed place or
    // size because a window opened, closed, floated or docked, or because the panes came back from
    // one window. A window that keeps its pane keeps its zoom and scroll, as when a tab brings its
    // window into the last pane of a 2-up to 6-up layout, or when 6-Up loses its sixth document.
    // Windows are compared by their slots, since a closed document shifts the indices of the
    // documents after it, and panes by their place in the layout.
    let before = std::mem::take(&mut app.canvas.panes.last_slots);
    let now: Vec<Option<Slot>> = shown.iter().map(|w| slot(app, *w)).collect();
    let unit = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1.0, 1.0));
    let (cells_before, cells_now) = (layout_cells(layout, before.len(), unit), layout_cells(layout, now.len(), unit));
    for (i, (w, s)) in shown.iter().zip(&now).enumerate() {
        if s.is_some() && before.get(i) == Some(s) && cells_before.get(i) == cells_now.get(i) {
            continue;
        }
        if let Some(v) = crate::doc_windows::view_mut(app, *w) {
            v.fitted = false;
        }
    }
    app.canvas.panes.last_slots = now;
    let t = Tokens::get(ui.ctx());
    let ppp = ui.ctx().pixels_per_point();
    let cells: Vec<(Win, egui::Rect)> = shown.iter().copied().zip(layout_cells(layout, shown.len(), ui.available_rect_before_wrap())).collect();
    let areas: Vec<(Win, egui::Rect)> = cells.iter().map(|(w, cell)| (*w, view_rects(app, *cell, ppp).1)).collect();
    let unfitted = refit_resized(app, &areas);
    let mut activate = None;
    let (mut cursor, mut dropped) = (None, None);
    // The other panes first, so that what the canvas draws past its pane (the Contextual Task Bar)
    // stays over them.
    for ((w, cell), (_, area)) in cells.iter().copied().zip(areas.iter().copied()).filter(|((w, _), _)| Some(*w) != active) {
        if pane(app, ui, w, cell) {
            activate = Some(w);
        }
        match artwork_over(app, ui, w, area) {
            Some(Over::Held(c)) => cursor = Some(c),
            Some(Over::Released(at, carried)) => dropped = Some((w, at, carried)),
            None => {}
        }
        ui.painter().rect_stroke(cell, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    }
    if let Some((_, cell)) = cells.iter().copied().find(|(w, _)| Some(*w) == active) {
        let mut canvas = ui.new_child(egui::UiBuilder::new().max_rect(cell));
        super::show(app, &mut canvas);
        ui.painter().rect_stroke(cell, 0.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
    }
    // After the canvas, which sets the cursor of the drag it holds.
    if let Some(c) = cursor {
        ui.ctx().set_cursor_icon(c);
    }
    // Note the fits this frame made, for `refit_resized`.
    for ((w, area), was_unfitted) in areas.iter().zip(unfitted) {
        if let (true, Some(v), Some(s)) = (was_unfitted, crate::doc_windows::view(app, *w).filter(|v| v.fitted).copied(), slot(app, *w)) {
            app.canvas.panes.fits.insert(s, (area.size(), v.zoom, v.center));
        }
    }
    app.canvas.panes.last = cells;
    if let Some((w, at, carried)) = dropped {
        crate::document_drag::drop_on_pane(app, ui.ctx(), w, at, carried);
    } else if let Some(w) = activate {
        crate::doc_windows::show_window(app, w);
    }
    true
}

/// A Selection drag's artwork over another window's tiled pane.
enum Over {
    /// Held there: the cursor that shows whether a release moves or copies it.
    Held(egui::CursorIcon),
    /// Released there, at a document point in the pane's own view.
    Released(Point, crate::document_drag::Carried),
}

/// A Selection drag's artwork over `area`, the part of window `w`'s pane inside its rulers and
/// scroll bars, where no panel or floating window is over the pointer. A release there takes the
/// artwork before the canvas sees the release ([`crate::document_drag::take_for_pane`]).
fn artwork_over(app: &mut VectorcraftApp, ui: &Ui, w: Win, area: egui::Rect) -> Option<Over> {
    let (uid, _) = slot(app, w)?;
    let p = ui.input(crate::document_drag::released_at)?;
    if !area.contains(p) || ui.ctx().layer_id_at(p).is_some_and(|l| l != ui.layer_id()) {
        return None;
    }
    let cursor = crate::document_drag::pane_cursor(app, ui.ctx(), uid)?;
    if !ui.input(|i| i.pointer.primary_released()) {
        return Some(Over::Held(cursor));
    }
    let v = crate::doc_windows::view(app, w).copied()?;
    let at = Xf::at(area, &v, ui.ctx().zoom_factor()).to_doc(p);
    Some(Over::Released(at, crate::document_drag::take_for_pane(app, ui.ctx())?))
}

/// Note window `w`'s pane, drawn in `cell` of `ui` outside Tile (a floating window inside the main
/// window, or the main window while the current window floats), as a place a Selection drag's
/// artwork can be released in ([`release_on_noted`]): the part inside its rulers and scroll bars.
pub(crate) fn note_drop(app: &mut VectorcraftApp, ui: &Ui, w: Win, cell: egui::Rect) {
    let Some(s) = slot(app, w) else { return };
    let area = view_rects(app, cell, ui.ctx().pixels_per_point()).1;
    app.canvas.windows.drops.push((s, area, ui.layer_id()));
}

/// The noted pane ([`note_drop`]) at `p`, where no panel or other window is over it: its window
/// and its art area.
fn noted_at(app: &VectorcraftApp, ctx: &egui::Context, p: egui::Pos2) -> Option<(Win, egui::Rect)> {
    let top = ctx.layer_id_at(p);
    let (s, area, _) = app.canvas.windows.drops.iter().rev().find(|(_, area, layer)| area.contains(p) && top.is_none_or(|l| l == *layer))?;
    let w = crate::doc_windows::windows(app).into_iter().find(|w| slot(app, *w) == Some(*s))?;
    Some((w, *area))
}

/// A Selection drag's artwork released over a pane the last frame noted ([`note_drop`]) drops into
/// its document ([`crate::document_drag::drop_on_pane`]), at the document point where the button
/// went up, in that pane's own view. Runs at the start of a frame, before any canvas sees the release.
pub(crate) fn release_on_noted(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.canvas.windows.drops.is_empty() || !ctx.input(|i| i.pointer.primary_released()) {
        return;
    }
    let Some(p) = ctx.input(crate::document_drag::released_at) else { return };
    let Some((w, area)) = noted_at(app, ctx, p) else { return };
    let Some(v) = crate::doc_windows::view(app, w).copied() else { return };
    let at = Xf::at(area, &v, ctx.zoom_factor()).to_doc(p);
    if let Some(carried) = crate::document_drag::take_for_pane(app, ctx) {
        crate::document_drag::drop_on_pane(app, ctx, w, at, carried);
    }
}

/// While a Selection drag's artwork is held over a pane this frame noted ([`note_drop`]), the cursor
/// shows whether a release there moves or copies it ([`crate::document_drag::pane_cursor`]).
pub(crate) fn noted_cursor(app: &VectorcraftApp, ctx: &egui::Context) {
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    if let Some((w, _)) = noted_at(app, ctx, p)
        && let Some((uid, _)) = slot(app, w)
        && let Some(c) = crate::document_drag::pane_cursor(app, ctx, uid)
    {
        ctx.set_cursor_icon(c);
    }
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
/// view with this too ([`crate::doc_windows`]). The document's colors, its paper and transparency
/// grid included, draw through its own assigned profiles, not the front document's.
pub(crate) fn pane(app: &mut VectorcraftApp, ui: &Ui, w: Win, cell: egui::Rect) -> bool {
    let Some(st) = app.session.documents().get(w.0) else { return false };
    let cms = vectorcraft_engine::cmd::colormgmt::doc_cms(&st.doc);
    vectorcraft_color::cms::with_active(cms, || draw_pane(app, ui, w, cell))
}

/// [`pane`], drawn through the color settings in place.
fn draw_pane(app: &mut VectorcraftApp, ui: &Ui, w: Win, cell: egui::Rect) -> bool {
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
    // View › Pixel Preview, as the canvas draws it.
    let max_side = ui.ctx().input(|i| i.max_texture_side);
    let pixel = (app.ui.view.pixel_preview && xf.zoom * ppp as f64 > 1.0).then(|| pixel_region(&xf, max_side)).flatten();
    // The pane's frame fits the largest texture side the display supports, as the canvas's does.
    let ppp = frame_ppp(rect.size(), ppp, max_side);
    let size = |l: f32| ((l * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX));
    let (pw, ph) = (size(rect.width()), size(rect.height()));
    let smooth_images = pixel.is_none() || app.session.prefs.anti_aliased_bitmaps;
    let plugins = plugin_key(&mut app.canvas.plugin_keys, &app.session, st);
    let placed = placed_key(&mut app.canvas.placed_keys, &app.session, st);
    let drawn = v.on_screen(ui_zoom);
    let mut key = cache_key(app, ui, st, &t, &drawn, (pw, ph, ppp), (pixel, smooth_images, plugins, placed));
    // The current document's drag and Image Trace hold are not this document's, and this view has
    // its own Outline or Preview.
    (key.dragging, key.trace_source, key.outline) = (false, None, v.outline);
    key.backdrop = backdrop(app, st, v.outline);
    let opts = frame_options(app, &key, st.shown_mask(), smooth_images);
    let (fw, fh, view) = frame_view(pixel, (pw, ph), &drawn, ppp);
    let doc = st.doc.clone();
    let display = app.display_profile.transform(app.session.prefs.monitor_profile_display);
    app.canvas.panes.draw(ui.ctx(), &painter, &xf, slot, (key, doc, ((fw, fh), view), opts), display);
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
        selected_objects(&mut app.canvas.panes, slot, st, &painter, &xf);
    } else {
        app.canvas.panes.outlines.remove(&slot);
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

/// Draw the outlines of the objects selected in `st` ([`outline_shapes`]) in `slot`'s pane, from
/// the outlines `panes` kept for the slot while they show the same revision, document, selection
/// and screen mapping, and made again otherwise.
fn selected_objects(panes: &mut Panes, slot: Slot, st: &vectorcraft_engine::DocState, painter: &egui::Painter, xf: &Xf) {
    let affine = xf.affine().as_coeffs();
    if !panes.outlines.get(&slot).is_some_and(|o| o.shows(st, &affine)) {
        let shapes = outline_shapes(st, xf);
        let kept = Outlines { revision: st.revision, doc: Arc::downgrade(&st.doc), selection: st.selection.objects.clone(), affine, shapes };
        panes.outlines.insert(slot, kept);
        panes.outlines_made += 1;
    }
    for shape in panes.outlines.get(&slot).map_or(&[][..], |o| &o.shapes) {
        painter.add(shape.clone());
    }
}

/// The outlines of the objects selected in `st` as lines on the screen through `xf`, in their
/// layers' colors; their bounds when they have too many points to outline.
fn outline_shapes(st: &vectorcraft_engine::DocState, xf: &Xf) -> Vec<egui::Shape> {
    let mut shapes = vec![];
    if st.selection.objects.is_empty() {
        return shapes;
    }
    let symbols = st.doc.symbol_boxes();
    let nodes: Vec<(vectorcraft_doc::NodeId, &vectorcraft_doc::Node)> =
        st.selection.objects.iter().filter_map(|id| Some((*id, st.doc.node(*id)?))).collect();
    let color = |id: vectorcraft_doc::NodeId| Stroke::new(1.0, c32(st.doc.layer_color(id)));
    let mut stroke = |bp: &BezPath, s: Stroke| shapes.extend(to_screen_path(bp, xf).into_iter().map(|line| egui::Shape::line(line, s)));
    if too_many_anchors(nodes.iter().map(|(_, n)| *n)) {
        let bounds = nodes.iter().filter_map(|(_, n)| n.geometric_bounds_with(&symbols)).reduce(|a, b| a.union(b));
        if let (Some(b), Some((id, _))) = (bounds, nodes.first()) {
            stroke(&vectorcraft_geom::shapes::rectangle(b).to_bezpath(), color(*id));
        }
        return shapes;
    }
    for (id, n) in nodes {
        stroke(&node_outline(n, &symbols), color(id));
    }
    shapes
}

#[cfg(test)]
mod tests;
