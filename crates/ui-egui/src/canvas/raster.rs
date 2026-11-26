use craft::behavior::{CanvasView, PixelSize};
use craft::platform::clock::monotonic_ms;
use craft::theme::Tokens;
use craft::ui::canvas::{AppPaint, CanvasSpace, RasterDisplay};
use craft::ui::contract::Show as _;
use craft::ui::primitives::ContentPainterCx;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::frame;
use egui::{Color32, Ui, pos2, vec2};
use vectorcraft_geom::{Affine, Point, Rect};

use super::artboards::{artboard_paper, paper_color, shows_artboards, shows_transparency_grid};
use super::cursor::{STILL_SECS, still_wait};
use super::fill::fill_uncovered;
use super::grid::textured_quad;
use super::guides::guide_color;
use super::{BodyKind, in_gesture};
use crate::doc_windows::Win;
use crate::{CacheKey, VectorcraftApp};

/// The rendered raster `img` as an egui image. Both are premultiplied RGBA8, so its bytes are copied
/// over whole instead of pixel by pixel, which costs milliseconds for a canvas on a large screen.
/// A raster whose bytes don't match its size gives an empty image of that size.
pub(crate) fn color_image(img: &vectorcraft_render::Rendered) -> egui::ColorImage {
    let size = [img.width as usize, img.height as usize];
    let expected = size[0].saturating_mul(size[1]).saturating_mul(4);
    if expected == img.pixels.len() {
        let pixels: Vec<egui::Color32> = bytemuck::cast_slice(&img.pixels).to_vec();
        egui::ColorImage::new(size, pixels)
    } else {
        egui::ColorImage::new(size, vec![egui::Color32::TRANSPARENT; size[0].saturating_mul(size[1])])
    }
}

/// Convert an owned [`vectorcraft_render::Rendered`] raster directly into an [`egui::ColorImage`].
/// Reuses the raster's heap allocation in place via zero-copy buffer casting when size and
/// alignment permit, eliminating redundant multi-megabyte allocations on texture upload.
pub(crate) fn color_image_owned(img: vectorcraft_render::Rendered) -> egui::ColorImage {
    let size = [img.width as usize, img.height as usize];
    let expected = size[0].saturating_mul(size[1]).saturating_mul(4);
    if expected == img.pixels.len() {
        let pixels: Vec<egui::Color32> = bytemuck::allocation::try_cast_vec(img.pixels).unwrap_or_else(|(_, v)| bytemuck::cast_slice(&v).to_vec());
        egui::ColorImage::new(size, pixels)
    } else {
        egui::ColorImage::new(size, vec![egui::Color32::TRANSPARENT; size[0].saturating_mul(size[1])])
    }
}

/// The pixels of the `w`×`h` frame through `view` to render again for `doc`, when only the document
/// changed, or placed documents' bitmaps were made, since the frame shown (its key `key` but for
/// the revision and [`CacheKey::placed`], and its render options `opts`), which this thread
/// rendered, and no frame is on its way from the worker: those it changed in, and those the
/// objects showing those bitmaps' files cover (`Some(None)`: none), grown to whole eighths of the
/// frame, as long as they are at most half of it. None: render the whole frame. Frames the worker
/// renders stay whole: its renderer keeps the rasters of the art's effects, this thread's doesn't.
/// What changed is measured by this thread's renderer, which drew the frame shown: it keeps what
/// it measures (a 3D effect's art) to draw the part again.
pub(super) fn changed_pixels(
    app: &mut VectorcraftApp,
    key: &CacheKey,
    doc: &vectorcraft_doc::Document,
    opts: &vectorcraft_render::RenderOptions,
    view: Affine,
    w: u32,
    h: u32,
) -> Option<Option<[u32; 4]>> {
    let (shown, _) = app.canvas.shown.as_ref().filter(|(_, o)| o == opts)?;
    // A viewed opacity mask shows its coverage wherever its art reaches, past its object's bounds.
    if opts.mask_view.is_some() {
        return None;
    }
    let shown_key = app.canvas.key.as_ref()?;
    let same_view = *shown_key == CacheKey { revision: shown_key.revision, placed: shown_key.placed, ..key.clone() };
    if !same_view || app.canvas.texture.is_none() || app.canvas.from_worker || app.canvas.worker.as_ref().is_some_and(|w| w.pending.is_some()) {
        return None;
    }
    // The placed documents' files whose bitmaps were made since the frame shown.
    let entry = app.session.documents().iter().find(|d| d.uid as usize == key.doc).and_then(|d| app.canvas.placed_keys.get(&d.uid));
    let placed = placed_changes(entry, shown_key.placed, key.placed, doc, vectorcraft_render::placed_document::made_between)?;
    match app.canvas.renderer.changed(shown, doc, opts).and(placed) {
        vectorcraft_render::Changed::Nothing => Some(None),
        vectorcraft_render::Changed::Area(area) => match vectorcraft_render::view_pixels(area, view, w, h) {
            None => Some(None),
            Some([x0, y0, x1, y1]) => {
                let ((x0, x1), (y0, y1)) = (in_steps(x0, x1, w), in_steps(y0, y1, h));
                (u64::from(x1 - x0) * u64::from(y1 - y0) * 2 <= u64::from(w) * u64::from(h)).then_some(Some([x0, y0, x1, y1]))
            }
        },
        vectorcraft_render::Changed::Everything => None,
    }
}

/// `a..b` of `0..n`, grown to whole eighths of `n` and moved back into `0..n`: the parts of a
/// frame rendered again come in few sizes, which keep the contexts the renderer keeps.
fn in_steps(a: u32, b: u32, n: u32) -> (u32, u32) {
    let step = n.div_ceil(8).max(1);
    let len = (b - a).max(1).div_ceil(step).saturating_mul(step).min(n);
    let a = a.min(n - len);
    (a, a + len)
}

/// The most bytes of the last frame the render worker keeps: 16 million pixels. A larger frame is
/// not kept, and every frame after it renders whole.
const MAX_KEPT_BYTES: usize = 64 << 20;

/// How far the view of a frame may be from a whole device pixel, in device pixels, for the frame's
/// pixels to be copied.
const WHOLE_PIXEL: f64 = 1e-6;

/// The render worker's last frame, which its next frame can copy pixels from when it holds them.
pub(crate) struct KeptFrame {
    key: CacheKey,
    opts: vectorcraft_render::RenderOptions,
    view: Affine,
    pub(crate) pixels: Option<Vec<u8>>,
}

/// A frame the render worker finished.
pub(crate) struct WorkerFrame {
    pub(crate) img: vectorcraft_render::Rendered,
    /// [`vectorcraft_render::FrameStats::moved`] of the renders that made the frame.
    pub(crate) moved: usize,
    /// The frame holds pixels copied from the last frame.
    pub(crate) reused: bool,
}

/// Nothing but the position of the view differs between the frame `kept` and the job `job`, in
/// their keys, render options and views, and the frame is not Pixel Preview's and shows no opacity
/// mask.
fn only_position_differs(kept: &KeptFrame, job: &crate::render_worker::Job) -> bool {
    let (a, b) = (&kept.key, &job.key);
    let same_scale = a.zoom == b.zoom && a.rot == b.rot && a.ppp == b.ppp && (a.w, a.h) == (b.w, b.h) && (job.w, job.h) == (b.w, b.h);
    same_scale
        && super::cursor::only_view_differs(a, b)
        && a.pixel.is_none()
        && b.pixel.is_none()
        && kept.opts == job.opts
        && job.opts.mask_view.is_none()
        && kept.view.as_coeffs()[..4] == job.view.as_coeffs()[..4]
}

/// How many whole device pixels the art moves between the frame `kept` and the job `job`, when
/// `job` can copy pixels from it: `job` allows it, nothing but the position of the view differs
/// ([`only_position_differs`]), and the art moves by whole device pixels.
fn whole_pixel_pan(kept: &KeptFrame, job: &crate::render_worker::Job) -> Option<(i64, i64)> {
    if !job.may_copy || !only_position_differs(kept, job) {
        return None;
    }
    let (from, to) = (kept.view.as_coeffs(), job.view.as_coeffs());
    let whole = |d: f64| (d.is_finite() && (d - d.round()).abs() <= WHOLE_PIXEL && d.abs() < f64::from(u16::MAX)).then_some(d.round() as i64);
    Some((whole(to[4] - from[4])?, whole(to[5] - from[5])?))
}

/// The parts `[x0, y0, x1, y1]` of a `w`×`h` frame that a pan by `(dx, dy)` device pixels exposes,
/// grown to whole eighths of the frame. None when they cover more than half of it.
fn exposed_parts(w: u32, h: u32, (dx, dy): (i64, i64)) -> Option<Vec<[u32; 4]>> {
    // How much of 0..n a move by `d` exposes, at the start for a positive `d`, else at the end.
    let span = |d: i64, n: u32| {
        let len = u32::try_from(d.unsigned_abs()).ok()?.min(n);
        Some(in_steps(if d > 0 { 0 } else { n - len }, if d > 0 { len } else { n }, n))
    };
    let mut parts = Vec::with_capacity(2);
    if dx != 0 {
        let (x0, x1) = span(dx, w)?;
        parts.push([x0, 0, x1, h]);
    }
    if dy != 0 {
        let (y0, y1) = span(dy, h)?;
        parts.push([0, y0, w, y1]);
    }
    let area: u64 = parts.iter().map(|[x0, y0, x1, y1]| u64::from(x1 - x0) * u64::from(y1 - y0)).sum();
    (area * 2 <= u64::from(w) * u64::from(h)).then_some(parts)
}

/// `pixels` of a `w`×`h` frame with the art moved by `(dx, dy)` device pixels. The pixels the art
/// does not reach stay transparent.
fn moved_pixels(pixels: &[u8], w: u32, h: u32, (dx, dy): (i64, i64)) -> Vec<u8> {
    let (w, h) = (i64::from(w), i64::from(h));
    let mut out = vec![0; pixels.len()];
    let (x0, x1) = (dx.max(0), (w + dx).min(w));
    if x1 > x0 {
        for y in dy.max(0)..(h + dy).min(h) {
            let (from, to, n) = (((y - dy) * w + x0 - dx) as usize * 4, (y * w + x0) as usize * 4, (x1 - x0) as usize * 4);
            out[to..to + n].copy_from_slice(&pixels[from..from + n]);
        }
    }
    out
}

/// The frame of `job` from `pixels`, the last frame's moved by the pan, and the exposed `parts`
/// rendered on their own, with its [`vectorcraft_render::FrameStats::moved`]. None when a part draws
/// art in a way that depends on the frame's size ([`vectorcraft_render::FrameStats::view_sized`]).
fn composed_frame(
    r: &mut vectorcraft_render::Renderer,
    mut pixels: Vec<u8>,
    parts: Vec<[u32; 4]>,
    job: &crate::render_worker::Job,
) -> Option<(vectorcraft_render::Rendered, usize)> {
    let mut moved = 0;
    let stride = job.w as usize * 4;
    for [x0, y0, x1, y1] in parts {
        let view = Affine::translate((-f64::from(x0), -f64::from(y0))) * job.view;
        let part = r.render(&job.doc, x1 - x0, y1 - y0, view, &job.opts);
        if r.stats.view_sized > 0 {
            return None;
        }
        moved += r.stats.moved;
        let row = (x1 - x0) as usize * 4;
        for (i, line) in part.pixels.chunks_exact(row).enumerate() {
            let to = (y0 as usize + i) * stride + x0 as usize * 4;
            pixels[to..to + row].copy_from_slice(line);
        }
    }
    Some((vectorcraft_render::Rendered { width: job.w, height: job.h, pixels }, moved))
}

/// Render the frame of `job`. A pan by whole device pixels copies the pixels that stay on screen
/// from `kept`, the frame before it, and renders only the parts it exposes. Any other frame renders
/// whole, and so does a pan when a part it exposes draws art in a way that depends on the frame's
/// size ([`vectorcraft_render::FrameStats::view_sized`]). A copied frame can differ from the same
/// frame rendered whole (by at most one level in the tests), so the canvas renders a frame whole
/// once it has been shown for [`super::STILL_SECS`] with the view unmoved
/// ([`crate::CanvasCache::reused`]). `kept` becomes this frame when `job` allows keeping it, it is
/// at most [`MAX_KEPT_BYTES`], it draws no raster from before its object moved, and it draws nothing
/// in a way that depends on its size. That whole frame is kept too, so the first step of the next
/// pan copies from it. A kept frame holds its pixels only when the worker kept no frame before it or
/// nothing but the position of the view differs from that frame. A frame after a zoom or an edit
/// holds none, so it costs no copy, and the first pan step after it renders whole.
pub(crate) fn render_job(r: &mut vectorcraft_render::Renderer, kept: &mut Option<KeptFrame>, job: &crate::render_worker::Job) -> WorkerFrame {
    let holds_pixels = kept.as_ref().is_none_or(|k| only_position_differs(k, job));
    let copied = kept.as_ref().and_then(|k| {
        let pan = whole_pixel_pan(k, job)?;
        let parts = exposed_parts(job.w, job.h, pan)?;
        composed_frame(r, moved_pixels(k.pixels.as_ref()?, job.w, job.h, pan), parts, job)
    });
    let reused = copied.is_some();
    let (mut img, moved) = copied.unwrap_or_else(|| {
        let img = r.render(&job.doc, job.w, job.h, job.view, &job.opts);
        let moved = r.stats.moved;
        (img, moved)
    });
    let keeps =
        job.may_keep && moved == 0 && r.stats.view_sized == 0 && img.pixels.len() <= MAX_KEPT_BYTES && (img.width, img.height) == (job.w, job.h);
    *kept =
        keeps.then(|| KeptFrame { key: job.key.clone(), opts: job.opts.clone(), view: job.view, pixels: holds_pixels.then(|| img.pixels.clone()) });
    // The kept frame stays sRGB, so the next frame's copied pixels are converted only once.
    let width = img.width;
    crate::display_profile::convert_over(job.display.as_deref(), &mut img.pixels, width, job.view, job.key.over());
    WorkerFrame { img, moved, reused }
}

/// Upload the art's raster directly into the canvas texture without reallocating or copying pixels;
/// Pixel Preview's document pixels keep hard edges when magnified.
pub(super) fn upload(app: &mut VectorcraftApp, ctx: &egui::Context, img: vectorcraft_render::Rendered, pixel: bool) {
    let color = color_image_owned(img);
    let options = if pixel { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
    match &mut app.canvas.texture {
        Some(tex) => tex.set(color, options),
        None => app.canvas.texture = Some(ctx.load_texture("canvas", color, options)),
    }
}

/// The device pixels a point that the art's frame of a canvas `size` points across renders at:
/// the display's `ppp`, lowered when a side of the frame would be more than `max_side` pixels, the
/// largest texture side the display supports (egui's `max_texture_side`). The frame is then drawn
/// scaled up over the canvas, at a lower resolution.
pub(super) fn frame_ppp(size: egui::Vec2, ppp: f32, max_side: usize) -> f32 {
    let longest = size.x.max(size.y) * ppp;
    let max = max_side.clamp(1, usize::from(u16::MAX)) as f32;
    if longest.round() <= max { ppp } else { ppp * (max / longest) }
}

/// Pixel Preview: the whole document pixels (x0, y0, x1, y1) under the canvas, rotated view
/// included. None when the view is degenerate, or when a side of the region is more than
/// `max_side` pixels, the largest texture side the display supports.
pub(super) fn pixel_region(view: &CanvasView, max_side: usize) -> Option<[i64; 4]> {
    let r = view.rect;
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| view.to_doc(p));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in corners {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.ceil(), y1.ceil());
    // A document pixel is bigger than a screen pixel here, so the region is about canvas-sized.
    let max = max_side.min(16384) as f64;
    let ok = [x0, y0, x1, y1].iter().all(|c| c.is_finite() && c.abs() < 1e9) && x1 > x0 && y1 > y0 && x1 - x0 <= max && y1 - y0 <= max;
    ok.then_some([x0 as i64, y0 as i64, x1 as i64, y1 as i64])
}

/// The canvas's artwork picture in `space`: `Live` renders the current document through the render
/// worker (or this thread) and draws the frame shown, reprojected while the next one is on its
/// way; `Pane` hands over to the pane's own picture.
pub(super) fn picture(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace, win: Win, kind: BodyKind) {
    if kind == BodyKind::Pane {
        return super::panes::pane_picture(app, ui, space, win);
    }
    let view = space.view;
    let rect = view.rect;
    let env = Env::of(ui);
    let Some(st) = app.session.active() else { return };
    let (doc, effects, mask_view) = (st.doc.clone(), st.plugin_effects.clone(), st.shown_mask());
    let device_ppp = ui.ctx().pixels_per_point();
    let max_side = ui.ctx().input(|i| i.max_texture_side);
    let ppp = frame_ppp(rect.size(), device_ppp, max_side);
    // Match Renderer::render_as before recording the cache extent or building its view.
    let (w, h) = (
        ((rect.width() * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX)),
        ((rect.height() * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX)),
    );
    // View › Pixel Preview: the art as it rasterizes, one pixel per point, while a document pixel
    // is bigger than a screen pixel (below that the screen render already shows it).
    let pixel = (app.ui.view.pixel_preview && view.zoom * device_ppp as f64 > 1.0).then(|| pixel_region(&view, max_side)).flatten();
    // File Handling › Display Bitmaps as Anti-aliased Images in Pixel Preview: off, images show
    // their pixels as they rasterize, hard-edged.
    let smooth_images = pixel.is_none() || app.session.prefs.anti_aliased_bitmaps;
    let plugins = plugin_key(&mut app.canvas.plugin_keys, &app.session, st);
    let placed = placed_key(&mut app.canvas.placed_keys, &app.session, st);
    let key = cache_key(app, ui.ctx(), st, env.tokens.tokens(), &view, (w, h, ppp), (pixel, smooth_images, plugins, placed));
    // Placed documents' bitmaps are being made: draw again when they are ready.
    if vectorcraft_render::placed_document::busy() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
    }
    poll_worker(app, ui, key.doc);
    // With `waits_for_still` set (the web build, which has no render worker), a heavy document is
    // not rendered while only the view changes: the frame shown is drawn reprojected, and the
    // first frame after the view has been still for STILL_SECS renders. An edit meanwhile is drawn
    // into the frame shown ([`edit_in_frame_shown`]).
    let wait = if app.canvas.waits_for_still {
        edit_in_frame_shown(app, &key, &doc, &effects, &view, (mask_view, smooth_images));
        let slow = app.canvas.last_ms > 8.0 && app.canvas.texture.is_some();
        still_wait(&mut app.canvas.still, app.canvas.key.as_ref(), &key, slow, frame::clock(ui.ctx()))
    } else {
        None
    };
    if let Some(secs) = wait {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(secs));
    }
    // A frame with copied pixels renders whole once it has been shown for STILL_SECS with the view unmoved.
    let same_frame = app.canvas.key.as_ref() == Some(&key);
    let settle = app.canvas.reused.filter(|_| same_frame).is_some_and(|at| {
        let waited = frame::clock(ui.ctx()) - at;
        // The clock went back (a new egui context): don't wait for it to catch up.
        let waiting = (0.0..STILL_SECS).contains(&waited);
        if waiting {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(STILL_SECS - waited));
        }
        !waiting
    });
    if wait.is_none() && (!same_frame || app.canvas.texture.is_none() || settle) {
        let (fw, fh, frame) = frame_view(pixel, (w, h), &view, ppp);
        let opts = frame_options(app, &key, mask_view, smooth_images);
        draw_frame(app, ui, Frame { key: &key, doc: &doc, effects: &effects, size: (fw, fh), view: frame, opts, pixel });
    }
    report_deferred(app, &effects);
    // The status bar reports each image that a renderer (this thread's or the worker's) could not
    // decode, once per image in the process.
    let undecoded = vectorcraft_render::newly_undecodable(doc.images.values());
    if undecoded > 0 {
        app.status(crate::i18n::msg_n(
            undecoded as u64,
            "{n} image couldn't be decoded and shows as a crossed box",
            "{n} images couldn't be decoded and show as crossed boxes",
            &[],
        ));
    }
    // The pages carry the transparency grid, so the display draws no checkerboard of its own.
    // PLAN: assumed the display's document space is the frame's pixels (Pixel Preview's region
    // size, else the canvas's frame size), as `RasterDisplay::painted` takes a `PixelSize`.
    let (pw, ph) = pixel.map_or((w, h), |[x0, y0, x1, y1]| ((x1 - x0) as u32, (y1 - y0) as u32));
    let size = PixelSize::new(pw.max(1), ph.max(1));
    let ctx = ui.ctx().clone();
    let paint = AppPaint::new(|cx: &ContentPainterCx<'_>, at: CanvasView| {
        fill_uncovered(app, &ctx, &cx.painter, &key, &doc, at.rect, &at);
        reproject_frame(app, &cx.painter, &at, at.rect, (key.doc, at.rotation));
    });
    RasterDisplay::painted(tl!("Artwork"), size, paint).picture().show_at(ui, rect);
}

/// What the cached frame depends on.
pub(super) fn cache_key(
    app: &VectorcraftApp,
    ctx: &egui::Context,
    st: &vectorcraft_engine::DocState,
    t: &Tokens,
    v: &CanvasView,
    (w, h, ppp): (u32, u32, f32),
    (pixel, smooth_images, plugins, placed): (Option<[i64; 4]>, bool, u64, u64),
) -> CacheKey {
    // Paths made into guides are drawn as the ruler guides are (Preferences › Guides & Grid).
    let [r, g, b, _] = guide_color(&app.session.prefs, t).0;
    CacheKey {
        guide: ([r, g, b], app.session.prefs.guide_style == "dots"),
        doc: st.uid as usize,
        revision: st.revision,
        interacting: st.interaction.is_some(),
        zoom: v.zoom,
        cx: v.center.x,
        cy: v.center.y,
        w,
        h,
        outline: app.outline_view(),
        trim: app.ui.view.trim_view,
        templates: app.ui.view.templates,
        guides: app.ui.view.guides_shown(),
        ppp,
        hidden: vec![],
        rot: v.rotation,
        anti_alias: app.session.prefs.anti_aliased_artwork,
        highlight_alternate_glyphs: app.session.prefs.highlight_alternate_glyphs,
        highlight_substituted_fonts: app.session.prefs.highlight_substituted_fonts,
        placed,
        plugins,
        pixel,
        smooth_images,
        isolated: st.isolation.map(|id| id.0),
        // Appearance of Black › On Screen: Display All Blacks as Rich Black.
        rich_black: app.session.prefs.black_on_screen == "rich",
        // A tool's drag on the canvas, during an interaction (not a dialog's preview).
        dragging: app.session.in_interaction() && in_gesture(app, ctx),
        trace_source: app.ui.trace_hold_source.clone(),
        // Preferences › Performance › Use Monitor Profile for Canvas.
        display: app.display_profile.key(app.session.prefs.monitor_profile_display),
        backdrop: backdrop(app, st, app.outline_view()),
        color: color_key(),
    }
}

/// [`CacheKey::color`] now: the active color settings, the working color settings and the loaded
/// profiles' generation.
fn color_key() -> (vectorcraft_color::cms::ColorSettings, vectorcraft_color::cms::ColorSettings, u64) {
    (vectorcraft_color::cms::active_settings(), vectorcraft_color::cms::working_settings(), vectorcraft_color::cms::profiles_generation())
}

/// [`CacheKey::plugins`] of document `st`, one of the documents open in `session`. `keys` holds each
/// document's value and the plug-in revision it was checked at, so a document is looked through only
/// once after each change of the plug-in revision.
pub(super) fn plugin_key(
    keys: &mut std::collections::HashMap<u64, (u64, u64)>,
    session: &vectorcraft_engine::Session,
    st: &vectorcraft_engine::DocState,
) -> u64 {
    let revision = vectorcraft_render::effects::plugin_revision();
    match keys.get(&st.uid) {
        Some(&(checked, key)) if checked == revision => key,
        seen => {
            let key = if vectorcraft_render::draws_plugin_effects(&st.doc) { revision } else { seen.map_or(0, |&(_, key)| key) };
            keys.retain(|uid, _| session.documents().iter().any(|d| d.uid == *uid));
            keys.insert(st.uid, (revision, key));
            key
        }
    }
}

/// [`CacheKey::placed`] of document `st`, one of the documents open in `session`. `keys` holds each
/// document's value and the placed-document bitmap generations it was checked at
/// ([`vectorcraft_render::placed_document::generation`]).
pub(super) fn placed_key(
    keys: &mut std::collections::HashMap<u64, crate::PlacedKey>,
    session: &vectorcraft_engine::Session,
    st: &vectorcraft_engine::DocState,
) -> u64 {
    placed_key_at(keys, session, st, vectorcraft_render::placed_document::generation(), vectorcraft_render::placed_document::made_between)
}

/// [`placed_key`] at generation `now`, with `made` giving the files whose bitmaps were made between
/// two generations ([`vectorcraft_render::placed_document::made_between`]). The value becomes `now`
/// when `st` shows a placed document of a file whose bitmap was made since the last check, or when
/// `made` no longer knows which files they were and `st` shows a placed document of a file in its
/// images, as [`vectorcraft_render::placed_area`] decides (every file shows while a pattern is
/// edited). Otherwise it stays: a bitmap made for another document, or for a file `st` keeps only
/// for Undo, leaves this document's frame as it is. A document checked for the first time takes
/// `now`.
pub(super) fn placed_key_at(
    keys: &mut std::collections::HashMap<u64, crate::PlacedKey>,
    session: &vectorcraft_engine::Session,
    st: &vectorcraft_engine::DocState,
    now: u64,
    made: impl FnOnce(u64, u64) -> Option<std::collections::HashSet<String>>,
) -> u64 {
    let entry = match keys.get(&st.uid).copied() {
        Some(p) if p.checked == now => return p.key,
        None => crate::PlacedKey { checked: now, key: now, before: (now, now) },
        Some(p) => {
            let images = &st.doc.images;
            // The files made that `st` holds: a placed document of one of them may show.
            let files: std::collections::HashSet<String> = if images.is_empty() {
                Default::default()
            } else {
                match made(p.checked, now) {
                    Some(files) => files.into_iter().filter(|f| images.contains_key(f)).collect(),
                    None => images.keys().cloned().collect(),
                }
            };
            let stays = matches!(vectorcraft_render::placed_area(&st.doc, &files), vectorcraft_render::Changed::Nothing);
            if stays { crate::PlacedKey { checked: now, ..p } } else { crate::PlacedKey { checked: now, key: now, before: (p.key, p.checked) } }
        }
    };
    keys.retain(|uid, _| session.documents().iter().any(|d| d.uid == *uid));
    keys.insert(st.uid, entry);
    entry.key
}

/// The generation after which the bitmaps made decide what a frame shown at [`CacheKey::placed`]
/// value `shown` draws again at value `key`, with `entry` the document's [`crate::PlacedKey`]. When
/// the value changed once since `shown`, it is the last generation `shown` was checked at: no
/// bitmap of the document's placed documents was made between `shown` and that generation.
/// Otherwise it is `shown`.
pub(super) fn placed_since(entry: Option<&crate::PlacedKey>, shown: u64, key: u64) -> u64 {
    entry.filter(|p| p.key == key && p.before.0 == shown).map_or(shown, |p| p.before.1)
}

/// What placed documents' bitmaps change in `doc` between a frame shown at [`CacheKey::placed`]
/// value `shown` and the frame at value `key`: where `doc` shows the files whose bitmaps were made
/// after [`placed_since`], with `entry` the document's [`crate::PlacedKey`] and `made` giving the
/// files whose bitmaps were made between two generations
/// ([`vectorcraft_render::placed_document::made_between`]). None when those files are no longer
/// known.
pub(super) fn placed_changes(
    entry: Option<&crate::PlacedKey>,
    shown: u64,
    key: u64,
    doc: &vectorcraft_doc::Document,
    made: impl FnOnce(u64, u64) -> Option<std::collections::HashSet<String>>,
) -> Option<vectorcraft_render::Changed> {
    if shown == key {
        return Some(vectorcraft_render::Changed::Nothing);
    }
    Some(vectorcraft_render::placed_area(doc, &made(placed_since(entry, shown, key), key)?))
}

/// Once the first view of a document has been drawn, the status bar says how many plug-in effect
/// stages it left deferred, if any.
fn report_deferred(app: &mut VectorcraftApp, effects: &vectorcraft_plugins::deferred::Deferrals) {
    if let Some(n) = effects.take_report().filter(|n| *n > 0) {
        app.status(crate::i18n::msg_n(
            n as u64,
            "Opened with {n} plug-in effect deferred. Effect › Refresh Deferred Effects runs it.",
            "Opened with {n} plug-in effects deferred. Effect › Refresh Deferred Effects runs them.",
            &[],
        ));
    }
}

/// Start the render worker, and upload the frame it finished.
pub(super) fn poll_worker(app: &mut VectorcraftApp, ui: &Ui, key_doc: usize) {
    if !app.canvas.worker_started {
        app.canvas.worker_started = true;
        if std::env::var_os("VECTORCRAFT_SYNC_RENDER").is_none() {
            app.canvas.worker = crate::render_worker::Worker::spawn(ui.ctx().clone());
            app.canvas.fill_worker = crate::render_worker::Worker::spawn(ui.ctx().clone());
        }
    }
    if let Some(done) = app.canvas.worker.as_mut().and_then(|w| w.poll()) {
        show_worker_frame(app, ui.ctx(), done, key_doc);
    }
}

/// Upload frame `done` from the render worker when it is of document `key_doc` and not older than
/// the frame shown. This prevents canvas freezes during continuous edits (drags, arrow nudges, MCP
/// command bursts) while ensuring obsolete out-of-order frames are dropped. A frame whose color key
/// ([`CacheKey::color`]) is not the current one is dropped too: the worker converted its colors
/// with the settings in force while it rendered, which may not be the ones in its key, and the next
/// frame renders again with the current ones.
fn show_worker_frame(app: &mut VectorcraftApp, ctx: &egui::Context, done: crate::render_worker::Done, key_doc: usize) {
    let not_older = app.canvas.key.as_ref().filter(|k| k.doc == done.key.doc).is_none_or(|k| done.key.revision >= k.revision);
    if done.key.doc != key_doc || !not_older || done.key.color != color_key() {
        return;
    }
    upload(app, ctx, done.img, done.key.pixel.is_some());
    if let Some(d) = app.session.documents().iter().find(|d| d.uid as usize == done.key.doc) {
        d.plugin_effects.first_view_done();
    }
    app.canvas.key = Some(done.key);
    app.canvas.shown = Some((done.doc, done.opts));
    app.perf.render_ms = done.ms;
    // A frame with copied pixels leaves `last_ms` at the whole frame's.
    if !done.reused {
        app.canvas.last_ms = done.ms;
    }
    app.canvas.reused = done.reused.then(|| frame::clock(ctx));
    app.canvas.moved = done.moved > 0;
    app.canvas.from_worker = true;
}

/// The size and view matrix of the frame to render: the pixel region in Pixel Preview, else the whole canvas.
pub(super) fn frame_view(pixel: Option<[i64; 4]>, (w, h): (u32, u32), v: &CanvasView, ppp: f32) -> (u32, u32, Affine) {
    match pixel {
        Some([x0, y0, x1, y1]) => ((x1 - x0) as u32, (y1 - y0) as u32, Affine::translate((-x0 as f64, -y0 as f64))),
        None => (
            w,
            h,
            Affine::translate((w as f64 / 2.0, h as f64 / 2.0))
                * Affine::rotate(v.rotation.to_radians())
                * Affine::scale(v.zoom * ppp as f64)
                * Affine::translate(-v.center.to_vec2()),
        ),
    }
}

/// The options the canvas renders the document with.
pub(super) fn frame_options(
    app: &VectorcraftApp,
    key: &CacheKey,
    mask_view: Option<vectorcraft_doc::NodeId>,
    smooth_images: bool,
) -> vectorcraft_render::RenderOptions {
    let opts = vectorcraft_render::RenderOptions { outline: key.outline, background: None, artboards: false, ..Default::default() };
    vectorcraft_render::RenderOptions {
        proof: vectorcraft_render::proof::active_proof(),
        overprint_preview: vectorcraft_render::proof::overprint_preview_on(),
        trim: app.ui.view.trim_view,
        hide_templates: !app.ui.view.templates,
        hide_guides: !app.ui.view.guides_shown(),
        tile_edge: vectorcraft_color::Color::from_hex(&app.session.prefs.pattern_tile_edge_color).map_or(opts.tile_edge, |c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            [r, g, b]
        }),
        guide: key.guide,
        mask_view,
        highlight_substitutions: true,
        // Type › Highlight Alternate Glyphs (#394): pink behind OpenType substitutions.
        highlight_alternate_glyphs: app.session.prefs.highlight_alternate_glyphs,
        // Type › Highlight Substituted Fonts: pink behind text in a substitute font.
        highlight_substituted_fonts: app.session.prefs.highlight_substituted_fonts,
        // General › Anti-aliased Artwork: off, edges are hard on screen (raster effects and
        // pattern tiles stay smooth).
        anti_alias: if app.session.prefs.anti_aliased_artwork { vectorcraft_render::AntiAlias::Art } else { vectorcraft_render::AntiAlias::None },
        progressive_placed: true,
        trace_views: true,
        // Image Trace › the eye button held: the source image of the selected objects.
        trace_source: key.trace_source.clone(),
        smooth_images,
        // Isolation mode: the art around the isolated group or layer is dimmed (#833).
        isolated: key.isolated.map(vectorcraft_doc::NodeId),
        // Images far smaller on screen than they are sample their mip levels (#1244).
        image_mips: true,
        // An image whose pixels can't be decoded shows as a crossed box, and the status message
        // below reports it.
        mark_undecoded_images: true,
        // During a drag or a dialog's preview, objects that only moved draw their raster
        // effects from the cached rasters of an earlier frame (#1257).
        interacting: key.interacting,
        // During a tool's drag, raster effects without a cached raster to draw render at a
        // lower resolution (#2558); the frame after the drag renders them as a still frame does.
        dragging: key.dragging,
        rich_black: key.rich_black,
        ..opts
    }
}

/// A frame to render: here, or on the worker.
struct Frame<'a> {
    key: &'a CacheKey,
    doc: &'a std::sync::Arc<vectorcraft_doc::Document>,
    /// The document's deferred plug-in effect stages: the frame renders inside their view.
    effects: &'a vectorcraft_plugins::deferred::Deferrals,
    size: (u32, u32),
    view: Affine,
    opts: vectorcraft_render::RenderOptions,
    pixel: Option<[i64; 4]>,
}

/// How long the UI thread waits for a document's first frame from the render worker before it
/// shows the document's paper without the art: about a frame at 60 Hz.
pub(crate) const FIRST_FRAME_GRACE: std::time::Duration = std::time::Duration::from_millis(16);

/// Draw `region` of `frame` again, into the texture of the frame shown, which then shows `frame`'s
/// key: only the document changed since the frame shown, in that part of it
/// ([`changed_pixels`]). `None`: nothing in view changed. `last_ms` stays the whole frame's.
fn draw_changed(app: &mut VectorcraftApp, frame: &Frame, region: Option<[u32; 4]>) {
    let Frame { key, doc, effects, view, opts, pixel, .. } = frame;
    let t0 = monotonic_ms();
    if region.is_none() {
        // Nothing in view to draw again: what was made to measure the edit isn't drawn.
        app.canvas.renderer.let_go_of_measured();
    }
    let display = app.display_profile.transform(app.session.prefs.monitor_profile_display);
    if let (Some([x0, y0, x1, y1]), Some(tex)) = (region, &mut app.canvas.texture) {
        let renderer = &mut app.canvas.renderer;
        let part_view = Affine::translate((-f64::from(x0), -f64::from(y0))) * *view;
        let mut part = effects.view(|| renderer.render(doc, x1 - x0, y1 - y0, part_view, opts));
        let width = part.width;
        crate::display_profile::convert_over(display.as_deref(), &mut part.pixels, width, part_view, key.over());
        let color = color_image(&part);
        let options = if pixel.is_some() { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
        tex.set_partial([x0 as usize, y0 as usize], color, options);
        app.canvas.moved |= app.canvas.renderer.stats.moved > 0;
    }
    app.canvas.key = Some((*key).clone());
    app.canvas.shown = Some(((*doc).clone(), opts.clone()));
    app.perf.render_ms = monotonic_ms() - t0;
}

/// With `waits_for_still` set (the web build) and the last render slow, an edit while the view
/// moves (a tool's drag that scrolls the view at an edge of the canvas, or a wheel scroll during a
/// drag) is drawn into the frame shown, at the view it was rendered for: the part the edit changed
/// only ([`changed_pixels`]). The frame shown then stays drawn reprojected with the view, and the
/// canvas renders whole once the view has been still for [`STILL_SECS`], as for a view change
/// without an edit (#8688). When more than the view and the document differ from the frame shown,
/// or the edit changed more than half of it, nothing is drawn here: the frame renders whole.
fn edit_in_frame_shown(
    app: &mut VectorcraftApp,
    key: &CacheKey,
    doc: &std::sync::Arc<vectorcraft_doc::Document>,
    effects: &vectorcraft_plugins::deferred::Deferrals,
    drawn: &CanvasView,
    (mask_view, smooth_images): (Option<vectorcraft_doc::NodeId>, bool),
) {
    let slow = app.canvas.last_ms > 8.0 && app.canvas.texture.is_some();
    let Some(shown) = app.canvas.key.clone().filter(|_| slow) else { return };
    let at_shown = CacheKey {
        zoom: shown.zoom,
        cx: shown.cx,
        cy: shown.cy,
        w: shown.w,
        h: shown.h,
        ppp: shown.ppp,
        rot: shown.rot,
        pixel: shown.pixel,
        ..key.clone()
    };
    if at_shown == shown || at_shown == *key || shown.pixel.is_some() != key.pixel.is_some() {
        return;
    }
    let mut v = *drawn;
    (v.center, v.zoom, v.rotation) = (Point::new(shown.cx, shown.cy), shown.zoom, shown.rot);
    let (w, h, view) = frame_view(at_shown.pixel, (at_shown.w, at_shown.h), &v, at_shown.ppp);
    let opts = frame_options(app, &at_shown, mask_view, smooth_images);
    if let Some(region) = changed_pixels(app, &at_shown, doc, &opts, view, w, h) {
        draw_changed(app, &Frame { key: &at_shown, doc, effects, size: (w, h), view, opts, pixel: at_shown.pixel }, region);
    }
}

fn draw_frame(app: &mut VectorcraftApp, ui: &Ui, frame: Frame) {
    let Frame { key, doc, effects, size: (w, h), view, opts, pixel } = frame;
    // Frames that would be slow on this thread (the last one took over 8 ms) go
    // to the worker. An interaction's frames stay on the worker once it drew the frame shown:
    // its renderer keeps the rasters of the art's effects that make those frames fast, and this
    // thread's renderer has none, so a frame fast on the worker would be slow here.
    let same_doc = app.canvas.key.as_ref().is_some_and(|k| k.doc == key.doc);
    let settling = app.canvas.moved && !key.interacting;
    let staying = key.interacting && app.canvas.from_worker;
    // A document's first frame (opened, or its tab chosen) renders on the worker too: how long it
    // takes is not known yet, and a heavy one froze the window for seconds (#4155). Its paper and
    // artboards show meanwhile.
    let first = app.canvas.texture.is_none() || !same_doc;
    let heavy = first || app.canvas.last_ms > 8.0 || settling || staying;
    let changed = changed_pixels(app, key, doc, &opts, view, w, h);
    // A new view can copy from the worker's last frame. The frame shown asked for again renders
    // whole. The worker keeps either one for the next pan to copy from.
    let may_copy = app.canvas.key.as_ref() != Some(key);
    let display = app.display_profile.transform(app.session.prefs.monitor_profile_display);
    let mut landed = None;
    match (&mut app.canvas.worker, heavy, changed) {
        // Only the document changed since the frame shown, in a part of it: that part is
        // rendered again, here, into the texture. `last_ms` stays the whole frame's.
        (_, _, Some(region)) => draw_changed(app, &Frame { key, doc, effects, size: (w, h), view, opts, pixel }, region),
        (Some(worker), true, None) => {
            // The worker draws the frame: what this thread's renderer made to measure the edit
            // isn't drawn here.
            app.canvas.renderer.let_go_of_measured();
            let queued = worker.submit(crate::render_worker::Job {
                key: key.clone(),
                doc: doc.clone(),
                w,
                h,
                view,
                opts,
                scope: vectorcraft_engine::file_access::ScopeHandle::capture(),
                effects: effects.clone(),
                may_copy,
                may_keep: true,
                display,
            });
            // A light document's first frame is ready within the grace and shows in this frame,
            // with no frame of bare paper. Waited for once, when the frame is queued.
            if first && queued {
                landed = worker.wait(FIRST_FRAME_GRACE);
            }
        }
        _ => {
            let t0 = monotonic_ms();
            let renderer = &mut app.canvas.renderer;
            let mut img = effects.view(|| renderer.render(doc, w, h, view, &opts));
            effects.first_view_done();
            let width = img.width;
            crate::display_profile::convert_over(display.as_deref(), &mut img.pixels, width, view, key.over());
            upload(app, ui.ctx(), img, pixel.is_some());
            app.canvas.key = Some(key.clone());
            app.canvas.shown = Some((doc.clone(), opts.clone()));
            app.perf.render_ms = monotonic_ms() - t0;
            app.canvas.last_ms = app.perf.render_ms;
            app.canvas.moved = app.canvas.renderer.stats.moved > 0;
            app.canvas.from_worker = false;
            app.canvas.reused = None;
        }
    }
    if let Some(done) = landed {
        show_worker_frame(app, ui.ctx(), done, key.doc);
    }
}

/// The paper [`super::paint_paper`] draws for document `st`, which translucent art is blended with
/// before the display profile's conversion. None without a display profile, and in Outline
/// (`outline`), whose white canvas is the interface's.
pub(super) fn backdrop(
    app: &VectorcraftApp,
    st: &vectorcraft_engine::DocState,
    outline: bool,
) -> Option<std::sync::Arc<crate::display_profile::Backdrop>> {
    if outline || app.display_profile.transform(app.session.prefs.monitor_profile_display).is_none() {
        return None;
    }
    let rgb = |c: Color32| [c.r(), c.g(), c.b()];
    let setup = &st.doc.setup;
    let paper = paper_color(setup);
    let b = if shows_artboards(app) {
        crate::display_profile::Backdrop {
            // In Overprint Preview the canvas around the artboards is white paper.
            everywhere: vectorcraft_render::proof::overprint_preview_on().then_some([255; 3]),
            artboards: st.doc.artboards.iter().map(|ab| (ab.rect, rgb(artboard_paper(ab, paper)))).collect(),
            grid: shows_transparency_grid(st).then(|| (setup.grid_colors.map(|c| rgb(crate::panels::c32(&c))), setup.grid_size.cell())),
        }
    } else {
        crate::display_profile::Backdrop { everywhere: Some(rgb(paper)), artboards: vec![], grid: None }
    };
    Some(std::sync::Arc::new(b))
}

/// The last rendered frame, placed through the current view `view` while a new one is on its way.
fn reproject_frame(app: &VectorcraftApp, painter: &egui::Painter, view: &CanvasView, rect: egui::Rect, (key_doc, rotation): (usize, f64)) {
    if let (Some(tex), Some(k)) = (&app.canvas.texture, &app.canvas.key)
        && k.doc == key_doc
    {
        // The frame being shown covers the canvas rect it was rendered for. While a panel is
        // dragged that rect is a different size, so reprojecting the frame through the *current*
        // rect stretches the art to the new size until the worker's frame arrives, then it snaps
        // back. Rebuild the rect it was rendered for from the key's own pixel size instead.
        let old_rect = {
            let ppp = if k.ppp > 0.0 { k.ppp } else { 1.0 };
            egui::Rect::from_center_size(rect.center(), vec2(k.w as f32 / ppp, k.h as f32 / ppp))
        };
        match k.pixel {
            // Document pixels: placed over the document rect they cover, turning with the view.
            Some([x0, y0, x1, y1]) => {
                textured_quad(painter, tex.id(), &view.quad(Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64)), (1.0, 1.0));
            }
            None if (k.rot - rotation).abs() < 1e-9 => {
                // Reproject the last frame if it was rendered for a different view.
                let old = CanvasView::new(old_rect, Point::new(k.cx, k.cy), k.zoom).rotation(rotation);
                let a = view.to_screen(old.to_doc(old_rect.min));
                let b = view.to_screen(old.to_doc(old_rect.max));
                painter.image(tex.id(), egui::Rect::from_min_max(a, b), egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            // The view turned after the last frame was rendered, and the worker renders the frame at
            // the new angle. Until it arrives, the last frame is drawn turned into this view: each
            // corner goes where this view puts the document point that the corner showed.
            None => {
                let old = CanvasView::new(old_rect, Point::new(k.cx, k.cy), k.zoom).rotation(k.rot);
                let q = [old_rect.left_top(), old_rect.right_top(), old_rect.right_bottom(), old_rect.left_bottom()]
                    .map(|c| view.to_screen(old.to_doc(c)));
                textured_quad(painter, tex.id(), &q, (1.0, 1.0));
            }
        }
    }
}
