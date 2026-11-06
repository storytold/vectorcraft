use super::{Affine, CacheKey, VectorcraftApp, Xf};

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
pub(super) fn changed_pixels(
    app: &VectorcraftApp,
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
    let placed = if shown_key.placed == key.placed {
        vectorcraft_render::Changed::Nothing
    } else {
        vectorcraft_render::placed_area(doc, &vectorcraft_render::placed_document::made_between(shown_key.placed, key.placed)?)
    };
    match vectorcraft_render::changed(shown, doc).and(placed) {
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

/// The render worker's last frame, which its next frame can copy pixels from.
pub(crate) struct KeptFrame {
    key: CacheKey,
    opts: vectorcraft_render::RenderOptions,
    view: Affine,
    pub(crate) pixels: Vec<u8>,
}

/// A frame the render worker finished.
pub(crate) struct WorkerFrame {
    pub(crate) img: vectorcraft_render::Rendered,
    /// [`vectorcraft_render::FrameStats::moved`] of the renders that made the frame.
    pub(crate) moved: usize,
    /// The frame holds pixels copied from the last frame.
    pub(crate) reused: bool,
}

/// How many whole device pixels the art moves between the frame `kept` and the job `job`, when
/// `job` can copy pixels from it: `job` allows it, nothing but the position of the view differs
/// between their keys and render options, the frame is not Pixel Preview's and shows no opacity
/// mask, and the art moves by whole device pixels.
fn whole_pixel_pan(kept: &KeptFrame, job: &crate::render_worker::Job) -> Option<(i64, i64)> {
    let (a, b) = (&kept.key, &job.key);
    let same_scale = a.zoom == b.zoom && a.rot == b.rot && a.ppp == b.ppp && (a.w, a.h) == (b.w, b.h) && (job.w, job.h) == (b.w, b.h);
    if !job.reuse || !same_scale || !super::cursor::only_view_differs(a, b) || a.pixel.is_some() || b.pixel.is_some() {
        return None;
    }
    if kept.opts != job.opts || job.opts.mask_view.is_some() {
        return None;
    }
    let (from, to) = (kept.view.as_coeffs(), job.view.as_coeffs());
    if from[..4] != to[..4] {
        return None;
    }
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

/// Render the frame of `job`. A pan by whole device pixels copies the pixels that stay on screen
/// from `kept`, the frame before it, and renders only the parts it exposes; any other frame renders
/// whole. A copied pixel can differ by one level from the same pixel rendered whole, because the
/// art's edges are computed from coordinates at the new position, so the canvas renders a frame
/// whole once the view has stopped ([`crate::CanvasCache::reused`]). `kept` becomes this frame
/// when `job` allows reuse, it is at most [`MAX_KEPT_BYTES`], and it draws no raster from before
/// its object moved.
pub(crate) fn render_job(r: &mut vectorcraft_render::Renderer, kept: &mut Option<KeptFrame>, job: &crate::render_worker::Job) -> WorkerFrame {
    let copied = kept.as_ref().and_then(|k| {
        let pan = whole_pixel_pan(k, job)?;
        let parts = exposed_parts(job.w, job.h, pan)?;
        Some((moved_pixels(&k.pixels, job.w, job.h, pan), parts))
    });
    let reused = copied.is_some();
    let (img, moved) = match copied {
        Some((mut pixels, parts)) => {
            let mut moved = 0;
            let stride = job.w as usize * 4;
            for [x0, y0, x1, y1] in parts {
                let view = Affine::translate((-f64::from(x0), -f64::from(y0))) * job.view;
                let part = r.render(&job.doc, x1 - x0, y1 - y0, view, &job.opts);
                moved += r.stats.moved;
                let row = (x1 - x0) as usize * 4;
                for (i, line) in part.pixels.chunks_exact(row).enumerate() {
                    let to = (y0 as usize + i) * stride + x0 as usize * 4;
                    pixels[to..to + row].copy_from_slice(line);
                }
            }
            (vectorcraft_render::Rendered { width: job.w, height: job.h, pixels }, moved)
        }
        None => {
            let img = r.render(&job.doc, job.w, job.h, job.view, &job.opts);
            (img, r.stats.moved)
        }
    };
    let keeps = job.reuse && moved == 0 && img.pixels.len() <= MAX_KEPT_BYTES && (img.width, img.height) == (job.w, job.h);
    *kept = keeps.then(|| KeptFrame { key: job.key.clone(), opts: job.opts.clone(), view: job.view, pixels: img.pixels.clone() });
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

/// Pixel Preview: the whole document pixels (x0, y0, x1, y1) under the canvas, rotated view
/// included. None when the view is degenerate.
pub(super) fn pixel_region(xf: &Xf) -> Option<[i64; 4]> {
    let r = xf.rect;
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| xf.to_doc(p));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in corners {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.ceil(), y1.ceil());
    // A document pixel is bigger than a screen pixel here, so the region is about canvas-sized.
    let ok = [x0, y0, x1, y1].iter().all(|c| c.is_finite() && c.abs() < 1e9) && x1 > x0 && y1 > y0 && x1 - x0 <= 16384.0 && y1 - y0 <= 16384.0;
    ok.then_some([x0 as i64, y0 as i64, x1 as i64, y1 as i64])
}
