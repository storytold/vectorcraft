//! Window › Arrange › Tile: every open document in a pane of its own, edge to edge in the document
//! area, in a grid, side by side or stacked; the 2-up to 6-up layouts show that many and leave the
//! rest under their tabs ([`Layout`]). The panes are the panes of a [`SplitViews`]. The current
//! document's pane is the live canvas; each other pane shows its document as a picture from a render
//! of its own ([`pane_picture`]), and a press in it makes its document the current one. Each view
//! Window › New Window opened has a pane of its own ([`crate::doc_windows`]).

use std::collections::HashMap;
use std::sync::Arc;

use craft::behavior::kurbo::Vec2;
use craft::behavior::{CanvasView, ZoomScheme};
use craft::ui::canvas::{CanvasSpace, SplitAt, SplitLayout, SplitViews};
use craft::ui::contract::{Bind, ShowContainer as _};
use craft::ui::primitives::textures::{LoadedPicture, Priority, Revision, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft::ui::runtime::env::Env;
use egui::Ui;
use serde_json::json;
use vectorcraft_geom::{Affine, Point};

use super::BodyKind;
use super::raster::{backdrop, cache_key, frame_options, frame_ppp, frame_view, pixel_region, placed_key, plugin_key};
use crate::doc_windows::Win;
use crate::state::View;
use crate::{CacheKey, VectorcraftApp};

/// Whose render a pane draws: its document's uid and its view's number.
type Slot = (u64, u32);

/// The margin, in points, a pane's fit leaves around the artboard.
const FIT_MARGIN: f64 = 30.0;

/// The key of a pane's picture in the [`Thumbnails`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct PaneKey {
    /// The document's uid and the view's number.
    slot: Slot,
}

/// What a pane's render needs, owned so the render can run off the UI thread.
struct PaneFrame {
    key: PaneKey,
    /// Fingerprint of everything the picture shows: the render's cache key.
    revision: Revision,
    cache: CacheKey,
    doc: Arc<vectorcraft_doc::Document>,
    /// The frame's size in pixels and the view it is rendered through.
    size: (u32, u32),
    view: Affine,
    opts: vectorcraft_render::RenderOptions,
    display: Option<Arc<craft::color_ui::display::DisplayTransform>>,
    cms: Arc<vectorcraft_color::cms::Cms>,
    /// Where the picture lies on the screen: the pane's art area, or the document pixels' rect in Pixel Preview.
    placed: egui::Rect,
}

/// The [`ThumbSource`] of one pane's frame.
pub(super) struct PaneSource {
    frame: PaneFrame,
}

impl ThumbSource<PaneKey> for PaneSource {
    fn revision(&self, _: &PaneKey, _: &ThumbRequest) -> Revision {
        self.frame.revision
    }

    fn render(&self, _: &PaneKey, _: &ThumbRequest) -> ThumbRender {
        let f = &self.frame;
        let (doc, opts, display, cms, cache, (w, h), view) =
            (f.doc.clone(), f.opts.clone(), f.display.clone(), f.cms.clone(), f.cache.clone(), f.size, f.view);
        ThumbRender::Later(Box::new(move |_| {
            let mut renderer = vectorcraft_render::Renderer::new();
            let img = render_pane(&mut renderer, (&doc, w, h, view, &opts), display.as_deref(), cms, cache.over());
            Ok(LoadedPicture::Image(super::raster::color_image_owned(img)))
        }))
    }
}

/// The thumbnails the panes' pictures live in.
fn thumbnails(ctx: &egui::Context) -> Thumbnails<PaneKey> {
    let max_side = ctx.input(|i| i.max_texture_side) as u32;
    Thumbnails::in_context(ctx, "vectorcraft.panes", ThumbnailOptions::new().max_side(max_side))
}

/// The windows' pane state of the tiled and floating windows other than the current one.
#[derive(Default)]
pub struct Panes {
    /// The panes the last frame drew, as (window, pane rect).
    pub(crate) last: Vec<(Win, egui::Rect)>,
    /// The slots of `last`'s windows, in the same order: which windows they were, as document
    /// indices shift when a document closes.
    last_slots: Vec<Option<Slot>>,
    /// By slot: the size of the view's area its view was last fitted to in a pane, and the zoom and
    /// center the fit gave.
    fits: HashMap<Slot, (egui::Vec2, f64, Point)>,
    /// Where the dividers between the tiled panes lie.
    at: SplitAt,
    /// While the tiled panes draw, the art areas of the panes drawn so far, by window: their drop catchers show after
    /// the current window's body ([`super::drops::show`]).
    pub(crate) drops: Option<Vec<(Win, egui::Rect)>>,
    /// Art released over a pane in the frame the pointer reached it, applied after the current window's body.
    pub(crate) released: Option<(Win, super::drops::Dropped)>,
    /// The `(uid, revision)` of the document whose carried art was released this pass, before the release gave up its
    /// move: the drop catchers that run after the release still take that art.
    pub(crate) released_from: Option<(u64, u64)>,
}

impl Panes {
    /// Forget every pane. The pictures go with the next frame that does not tile.
    fn clear(&mut self) {
        self.last.clear();
        self.last_slots.clear();
        self.fits.clear();
        self.at = SplitAt::default();
    }
}

/// Render `doc` (`w` × `h` pixels through `view`, with `opts`) through the color settings `cms`,
/// then convert it into the display profile `display`, blended with the backdrop `over` first
/// ([`crate::display_profile::convert_over`]).
fn render_pane(
    r: &mut vectorcraft_render::Renderer,
    (doc, w, h, view, opts): (&vectorcraft_doc::Document, u32, u32, Affine, &vectorcraft_render::RenderOptions),
    display: Option<&craft::color_ui::display::DisplayTransform>,
    cms: Arc<vectorcraft_color::cms::Cms>,
    over: Option<(&crate::display_profile::Backdrop, f64)>,
) -> vectorcraft_render::Rendered {
    let mut img = vectorcraft_color::cms::with_active(cms, || r.render(doc, w, h, view, opts));
    let width = img.width;
    crate::display_profile::convert_over(display, &mut img.pixels, width, view, over);
    img
}

impl PaneSource {
    /// The source of window `w`'s pane picture, rendered through `cv`, the view its canvas shows;
    /// `None` when the window has no document.
    pub(super) fn new(app: &mut VectorcraftApp, ctx: &egui::Context, w: Win, cv: &CanvasView) -> Option<PaneSource> {
        let env = Env::of_root(ctx);
        let ppp = ctx.pixels_per_point();
        let space = cv.rect;
        let v = crate::doc_windows::view(app, w).copied()?;
        let st = app.session.documents().get(w.0)?;
        // View › Pixel Preview, as the canvas draws it.
        let max_side = ctx.input(|i| i.max_texture_side);
        let pixel = (app.ui.view.pixel_preview && cv.zoom * f64::from(ppp) > 1.0).then(|| pixel_region(cv, max_side)).flatten();
        // The pane's frame fits the largest texture side the display supports, as the canvas's does.
        let ppp = frame_ppp(space.size(), ppp, max_side);
        let side = |l: f32| ((l * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX));
        let (pw, ph) = (side(space.width()), side(space.height()));
        let smooth_images = pixel.is_none() || app.session.prefs.anti_aliased_bitmaps;
        let plugins = plugin_key(&mut app.canvas.plugin_keys, &app.session, st);
        let placed = placed_key(&mut app.canvas.placed_keys, &app.session, st);
        let mut cache = cache_key(app, ctx, st, env.tokens.tokens(), cv, (pw, ph, ppp), (pixel, smooth_images, plugins, placed));
        // The current document's drag and Image Trace hold are not this document's, and this view has
        // its own Outline or Preview.
        (cache.dragging, cache.trace_source, cache.outline) = (false, None, v.outline);
        cache.backdrop = backdrop(app, st, v.outline);
        let opts = frame_options(app, &cache, st.shown_mask(), smooth_images);
        let (fw, fh, view) = frame_view(pixel, (pw, ph), cv, ppp);
        let on_screen = match pixel {
            Some([x0, y0, x1, y1]) => {
                egui::Rect::from_two_pos(cv.to_screen(Point::new(x0 as f64, y0 as f64)), cv.to_screen(Point::new(x1 as f64, y1 as f64)))
            }
            None => space,
        };
        let frame = PaneFrame {
            key: PaneKey { slot: (st.uid, w.1) },
            revision: Revision::of(&format!("{cache:?}")),
            cache,
            doc: st.doc.clone(),
            size: (fw, fh),
            view,
            opts,
            display: app.display_profile.transform(app.session.prefs.monitor_profile_display),
            cms: vectorcraft_engine::cmd::colormgmt::doc_cms(&st.doc),
            placed: on_screen,
        };
        Some(PaneSource { frame })
    }
}

/// Paint window `w`'s picture over its canvas `space` from the render of its document: the
/// previous picture shows until a new one lands.
pub(super) fn pane_picture(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace, w: Win) {
    let ctx = ui.ctx().clone();
    let Some(source) = PaneSource::new(app, &ctx, w, &space.view) else { return };
    let space = space.view.rect;
    let request = ThumbRequest::for_cell(space.width().max(space.height()), ctx.pixels_per_point());
    let (key, placed) = (source.frame.key, source.frame.placed);
    let state = thumbnails(&ctx).state(&ctx, &source, &key, &request, Priority::VISIBLE);
    if let Some(texture) = state.shown() {
        ui.painter_at(space).image(texture.id, placed, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
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
}

/// The arrangement `layout` lays `shown` panes out in: an n-up layout with fewer panes than cells
/// uses the smaller arrangement that holds them.
fn arrangement(layout: Layout, shown: usize) -> SplitLayout {
    match layout {
        Layout::Grid => SplitLayout::Tile,
        Layout::Vertical => SplitLayout::TileAllVertically,
        Layout::Horizontal => SplitLayout::TileAllHorizontally,
        Layout::Up(k) => match (k, shown) {
            (_, 0..=2) => SplitLayout::TwoUpVertical,
            (4, _) => SplitLayout::FourUp,
            (6, 4..) => SplitLayout::SixUp,
            _ => SplitLayout::ThreeUpVertical,
        },
    }
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
    app.canvas.panes.at = SplitAt::default();
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
        // The floating windows draw their panes from the same pictures.
        if app.ui.floating.is_empty() {
            thumbnails(ui.ctx()).retain(ui.ctx(), |_| false);
            app.canvas.panes.fits.clear();
        }
        app.canvas.panes.last.clear();
        app.canvas.panes.last_slots.clear();
        return false;
    }
    // The current window's pane is the canvas, unless it is a floating one.
    let active = crate::doc_windows::current(app).filter(|a| docked.contains(a));
    // The 2-up to 6-up layouts leave the windows past their panes under the tabs.
    let layout = app.ui.tile_layout;
    let shown = shown_windows(layout, &docked, active);
    app.sync_views();
    // The pictures of the panes and the floating windows stay; a window under its tab has none.
    let live: Vec<Slot> =
        crate::doc_windows::windows(app).into_iter().filter(|w| shown.contains(w) || !docked.contains(w)).filter_map(|w| slot(app, w)).collect();
    thumbnails(ui.ctx()).retain(ui.ctx(), |k| live.contains(&k.slot));
    app.canvas.panes.fits.retain(|s, _| live.contains(s));
    // A window that moved to another pane fits it, and so does a window whose pane changed place or
    // size because a window opened, closed, floated or docked, or because the panes came back from
    // one window. A window that keeps its pane keeps its zoom and scroll, as when a tab brings its
    // window into the last pane of a 2-up to 6-up layout, or when 6-Up loses its sixth document.
    // Windows are compared by their slots, since a closed document shifts the indices of the
    // documents after it, and panes by their place in the layout.
    let before = std::mem::take(&mut app.canvas.panes.last_slots);
    let now: Vec<Option<Slot>> = shown.iter().map(|w| slot(app, *w)).collect();
    let unit = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(1.0));
    let place = |count: usize| SplitAt::default().rects(arrangement(layout, count), count, unit);
    let (cells_before, cells_now) = (place(before.len()), place(now.len()));
    for (i, (w, s)) in shown.iter().zip(&now).enumerate() {
        if s.is_some() && before.get(i) == Some(s) && cells_before.get(i) == cells_now.get(i) {
            continue;
        }
        if let Some(v) = crate::doc_windows::view_mut(app, *w) {
            v.fitted = false;
        }
    }
    app.canvas.panes.last_slots = now;
    let arranged = arrangement(layout, shown.len());
    let active_pane = active.and_then(|a| shown.iter().position(|w| *w == a)).unwrap_or(0);
    let mut lines = std::mem::take(&mut app.canvas.panes.at);
    let placed = lines.clone();
    let mut cells: Vec<(Win, egui::Rect)> = vec![];
    let split = SplitViews::new("Documents", arranged, Bind::bound(&mut lines)).count(shown.len()).active(active_pane).show(ui, |ui, body| {
        cells = shown.iter().copied().zip(placed.placed(arranged, shown.len(), body.rect, body.env.direction().is_rtl())).collect();
        let areas: Vec<(Win, egui::Rect)> = cells.iter().map(|(w, cell)| (*w, art_area(app, *w, *cell))).collect();
        let unfitted = refit_resized(app, &areas);
        // The other panes first, so that what the canvas draws past its pane (the Contextual Task Bar)
        // stays over them; their drop catchers last, after the current window's tool may start a carry.
        app.canvas.panes.drops = Some(Vec::new());
        for (w, cell) in cells.iter().copied().filter(|(w, _)| Some(*w) != active) {
            pane(app, ui, w, cell);
        }
        if let Some((w, cell)) = cells.iter().copied().find(|(w, _)| Some(*w) == active) {
            show_body(app, ui, w, cell, BodyKind::Live);
        }
        for (w, area) in app.canvas.panes.drops.take().unwrap_or_default() {
            let mut catcher = ui.new_child(egui::UiBuilder::new().id_salt(("pane-drops", w)).max_rect(area));
            super::drops::show(app, &mut catcher, w, BodyKind::Pane);
        }
        drop_released(app);
        // Note the fits this frame made, for `refit_resized`.
        for ((w, area), was_unfitted) in areas.iter().zip(unfitted) {
            if let (true, Some(v), Some(s)) = (was_unfitted, crate::doc_windows::view(app, *w).filter(|v| v.fitted).copied(), slot(app, *w)) {
                app.canvas.panes.fits.insert(s, (area.size(), v.zoom, v.center));
            }
        }
    });
    let activated = split.output.activated;
    app.canvas.panes.at = lines;
    app.canvas.panes.last = cells;
    if let Some(&(w, _)) = activated.and_then(|i| app.canvas.panes.last.get(i)) {
        crate::doc_windows::show_window(app, w);
    }
    true
}

/// Drops the art released over a pane in the frame the pointer reached it ([`super::drops::released_on_pane`]), once
/// the panes that could have taken it as a drop have drawn.
pub(crate) fn drop_released(app: &mut VectorcraftApp) {
    app.canvas.panes.released_from = None;
    if let Some((w, drop)) = app.canvas.panes.released.take() {
        super::drops::drop_on_pane(app, w, &drop);
    }
}

/// Window `w`'s [`Slot`].
fn slot(app: &VectorcraftApp, (i, n): Win) -> Option<Slot> {
    app.session.documents().get(i).map(|d| (d.uid, n))
}

/// The area window `w`'s view shows its art in: what its body last showed, `cell` before the first.
fn art_area(app: &VectorcraftApp, w: Win, cell: egui::Rect) -> egui::Rect {
    crate::doc_windows::view(app, w).map(|v| v.rect).filter(|r| r.is_positive()).unwrap_or(cell)
}

/// Unfit each window whose view is still where a pane fit left it when its view's area (`areas`,
/// by window) has changed size, as when the window or a panel is resized: it fits its pane again.
/// A view zoomed or scrolled since keeps its place. → which of `areas`' views are unfitted now.
pub(crate) fn refit_resized(app: &mut VectorcraftApp, areas: &[(Win, egui::Rect)]) -> Vec<bool> {
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

/// Fit `v` to show its artboard (the first if it has gone) in `area`, with the interface zoomed by
/// `ui_zoom` on a screen of `pixels_per_point`, as Fit Artboard in Window does.
fn fit_pane(v: &mut View, doc: &vectorcraft_doc::Document, area: egui::Rect, (ui_zoom, pixels_per_point): (f32, f32), scheme: &ZoomScheme) {
    if let Some(target) = doc.artboards.get(v.artboard).or(doc.artboards.first()).map(|a| a.rect) {
        let mut view = v.canvas(ui_zoom, area);
        craft::behavior::canvas::zoom::fit_rect(&mut view, target, target.size(), pixels_per_point, Vec2::new(FIT_MARGIN, FIT_MARGIN), scheme);
        v.set_canvas(&view, ui_zoom);
        v.fitted = true;
    }
}

/// Fit window `w`'s view to its pane when it is unfitted, with the zoom stops of `ui`'s scope.
fn fit_window(app: &mut VectorcraftApp, ui: &Ui, w: Win, cell: egui::Rect) {
    let area = art_area(app, w, cell);
    let Some(doc) = app.session.documents().get(w.0).map(|st| st.doc.clone()) else { return };
    let scheme = Env::of(ui).get::<ZoomScheme>();
    let scaling = (app.ui_scaling(), ui.ctx().pixels_per_point());
    if let Some(v) = crate::doc_windows::view_mut(app, w).filter(|v| !v.fitted) {
        fit_pane(v, &doc, area, scaling, &scheme);
    }
}

/// Window `w`'s document body of `kind` in `cell`, its view fitted first. True when a press in its
/// canvas background asks for its window to become the current one.
fn show_body(app: &mut VectorcraftApp, ui: &mut Ui, w: Win, cell: egui::Rect, kind: BodyKind) -> bool {
    fit_window(app, ui, w, cell);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cell));
    super::show_document(app, &mut child, w, kind)
}

/// The pane of window `w` (document `i`'s view `n`), not the current one, in `cell`: its picture
/// with the rulers, guides and selection the body draws over it. True when a press in the pane asks
/// for its window to become the current one. A floating window shows its view with this too
/// ([`crate::doc_windows`]). The document's colors, its paper and transparency grid included, draw
/// through its own assigned profiles, not the front document's.
pub(crate) fn pane(app: &mut VectorcraftApp, ui: &mut Ui, w: Win, cell: egui::Rect) -> bool {
    let Some(st) = app.session.documents().get(w.0) else { return false };
    let cms = vectorcraft_engine::cmd::colormgmt::doc_cms(&st.doc);
    vectorcraft_color::cms::with_active(cms, || show_body(app, ui, w, cell, BodyKind::Pane))
}

#[cfg(test)]
mod tests;
