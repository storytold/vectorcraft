use super::*;

/// The rendered raster `img` as an egui image. Both are premultiplied RGBA8, so its bytes are copied
/// over whole instead of pixel by pixel, which costs milliseconds for a canvas on a large screen.
/// A raster whose bytes don't match its size gives an empty image of that size.
pub(crate) fn color_image(img: &vectorcraft_render::Rendered) -> egui::ColorImage {
    let size = [img.width as usize, img.height as usize];
    let mut pixels = vec![egui::Color32::TRANSPARENT; size[0] * size[1]];
    let bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut pixels);
    if bytes.len() == img.pixels.len() {
        bytes.copy_from_slice(&img.pixels);
    }
    egui::ColorImage::new(size, pixels)
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

/// Upload the art's raster; Pixel Preview's document pixels keep hard edges when magnified.
pub(super) fn upload(app: &mut VectorcraftApp, ctx: &egui::Context, img: &vectorcraft_render::Rendered, pixel: bool) {
    let color = color_image(img);
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
