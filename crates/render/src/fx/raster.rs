use super::entries::{Drawn, Inputs, Made, raster_bytes};
use super::{Affine, Document, EdgeMode, Filter, FilterPrimitive, Ink, RasterFx, Rect, RenderContext, Vec2, effects, peniko};

pub(super) fn shadow_filter(dx: f64, dy: f64, blur: f64, color: peniko::Color) -> Filter {
    Filter::from_primitive(FilterPrimitive::DropShadowOnly {
        dx: dx as f32,
        dy: dy as f32,
        std_deviation: (blur / 2.0).max(0.0) as f32,
        color,
        edge_mode: EdgeMode::None,
    })
}

/// The layer filter of a drop shadow with Darkness: source paints are adjusted before blur.
pub(super) fn shade_filter(blur: f64) -> Filter {
    Filter::from_primitive(FilterPrimitive::GaussianBlur { std_deviation: (blur / 2.0).max(0.0) as f32, edge_mode: EdgeMode::None })
}

/// What a shadow or glow raster is made from ([`Made::pixels`]): the silhouette's alpha, or its
/// premultiplied RGBA when the shadow takes the object's own colors (Darkness).
pub(super) fn silhouette(pm: &vello_cpu::Pixmap, colored: bool) -> Vec<u8> {
    if colored { pm.data().iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect() } else { pm.data().iter().map(|p| p.a).collect() }
}

pub(super) fn pcolor(ink: Ink, c: &vectorcraft_doc::color::Color) -> peniko::Color {
    let [r, g, b] = ink.rgb(c);
    peniko::Color::new([r, g, b, 1.0])
}

/// Most pixels the raster of [`Renderer::pixel_content`] holds (a 4K screen's): larger content is
/// cropped to the view when its effects only look nearby, else rendered at a lower resolution.
pub(super) const PIXEL_BUDGET: f64 = 8.4e6;

/// Most Rasterize effects of one stack that [`Renderer::raster_fx`] draws (the last ones): each
/// draws the content again inside the one after it.
pub(super) const MAX_RASTERIZE: usize = 16;

/// The columns and rows of the image Rasterize `r` makes over `rect` (whole pixels of the image,
/// `line` points wide at the document's raster effects resolution). `None` when the image would
/// hold more than [`PIXEL_BUDGET`] pixels or be wider or taller than [`crate::MAX_RASTER_SIDE`].
pub(super) fn image_size(r: &effects::Rasterize, rect: Rect, line: f64) -> Option<(u16, u16)> {
    let cell = r.cell(line);
    let (cols, rows) = ((rect.width() / cell).round(), (rect.height() / cell).round());
    let side = f64::from(crate::MAX_RASTER_SIDE);
    (cols >= 1.0 && rows >= 1.0 && cols <= side && rows <= side && cols * rows <= PIXEL_BUDGET).then_some((cols as u16, rows as u16))
}

#[cfg(test)]
thread_local! {
    /// How many times a Rasterize has drawn the content before it on this thread.
    pub(crate) static RASTERIZED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Document Raster Effects Settings › Resolution of `doc`, in ppi (1 to 2400; 72 when it isn't a
/// number).
pub(super) fn doc_raster_ppi(doc: &Document) -> f64 {
    doc.raster_ppi()
}

/// The effects [`Renderer::pixel_content`] runs on the content's pixels (those changing the
/// content itself), in stack order.
pub(super) fn content_changing(fx: &&RasterFx) -> bool {
    matches!(fx, RasterFx::Feather { .. } | RasterFx::GaussianBlur { .. } | RasterFx::Pixel(_) | RasterFx::Rasterize(_))
}

/// Is the content drawn with hard edges for `chain` into a raster of pixels `px` wide (document
/// units): does it go first through a Rasterize with anti-aliasing off whose pixels are no
/// smaller? (Smaller ones show as the share of each raster pixel they cover, as smooth edges do.)
pub(super) fn hard_edged(chain: &[&RasterFx], px: f64) -> bool {
    matches!(chain.first(), Some(RasterFx::Rasterize(r)) if !r.anti_alias && r.cell(72.0) >= px)
}

/// How far (document units) from a pixel the content deciding it after `fx` lies; `None` when
/// anywhere in the object.
pub(super) fn looks_around(fx: &RasterFx) -> Option<f64> {
    match fx {
        RasterFx::Feather { radius } | RasterFx::GaussianBlur { radius } => Some(1.5 * radius),
        RasterFx::Pixel(p) => p.reach(),
        RasterFx::Rasterize(r) => Some(r.reach()),
        _ => Some(0.0),
    }
}

/// The content of an object with Photoshop-style effects as filtered pixels, drawn at
/// (`anchor`'s view position + `rel`) scaled by 1 / `k` (see [`Renderer::pixel_content`]).
pub(crate) struct PixelEntry {
    pub(super) node: Drawn,
    pub(super) linear: [u64; 4],
    pub(super) anchor: vectorcraft_geom::Point,
    pub(super) rel: (f64, f64),
    pub(super) k: f64,
    pub(super) image: std::sync::Arc<vello_cpu::Pixmap>,
    /// For a raster of the content's whole reach at full size, `None`: it draws after a pan too.
    /// For one cropped to the view, or made at a lower resolution, the view and the rect it was
    /// made for: it draws only there, so dragging other art over the object keeps it (a wide blur,
    /// or any blur zoomed in, reaches past the pixel budget).
    pub(super) view: Option<ViewCrop>,
    pub(super) made: Made,
    /// What the content read when the raster was made, with the channels the effects ran on and,
    /// under De-Interlace, the raster resolution.
    pub(super) inputs: Inputs,
    /// Made for the object before it moved ([`Renderer::move_pixel_content`]), or made over
    /// rasters made before their objects moved: only a frame rendered while interacting draws
    /// it.
    pub(super) moved: bool,
    /// How many images in the content failed to decode while the raster was made
    /// ([`crate::paint::decode_failures`]): each frame that draws the raster counts them
    /// again ([`crate::paint::add_decode_failures`]).
    pub(super) failed: u64,
    pub(crate) stamp: u64,
}

impl PixelEntry {
    /// The bytes it holds, for [`FX_RASTER_BUDGET`].
    pub(crate) fn bytes(&self) -> usize {
        raster_bytes(&self.image) + self.made.bytes()
    }
}

/// The four linear coefficients of `view` (zoom and rotation, not the pan), as cache keys.
pub(super) fn linear_key(view: Affine) -> [u64; 4] {
    let c = view.as_coeffs();
    [c[0].to_bits(), c[1].to_bits(), c[2].to_bits(), c[3].to_bits()]
}

/// The pan of the view a raster was cropped for, or made at a lower resolution than the view's, and
/// the device-pixel rect it was made for in that view: the frame's, or around it the margin of
/// [`pan_margin`] ([`PixelEntry::view`], [`ShadowEntry::view`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct ViewCrop {
    pan: Vec2,
    screen: Rect,
}

impl ViewCrop {
    pub(super) fn of(view: Affine, screen: Rect) -> Self {
        Self { pan: view.translation(), screen }
    }

    /// Does the raster draw in a frame through `view` (of the same zoom and rotation) onto
    /// `screen`? When the view is panned by whole pixels and the frame's rect, panned back, lies
    /// within the one the raster was made for: the same pixels in the same places, as when a frame
    /// is drawn again only in a part of it.
    pub(super) fn serves(&self, view: Affine, screen: Rect) -> bool {
        let d = view.translation() - self.pan;
        let whole = Vec2::new(d.x.round(), d.y.round());
        let s = screen - whole;
        (d - whole).hypot() < 1e-6 && s.x0 >= self.screen.x0 && s.y0 >= self.screen.y0 && s.x1 <= self.screen.x1 && s.y1 <= self.screen.y1
    }

    /// The rect the raster was made for, in the device pixels of a frame through `view` that it
    /// serves ([`Self::serves`]).
    pub(super) fn served(&self, view: Affine) -> Rect {
        let d = view.translation() - self.pan;
        self.screen + Vec2::new(d.x.round(), d.y.round())
    }
}

/// How far (device pixels, on each side) past a `w` × `h` view a raster cropped to it, and `m`
/// pixels around that, is made when it is made again at the same zoom and rotation, so that later
/// pans of up to that many pixels draw it: the most that keeps the raster within `budget` pixels and
/// `side` pixels a side, with two pixels to spare for rounding, and at most half the view's longer
/// side. 0 when the view and the `m` pixels around it alone leave no room.
pub(super) fn pan_margin((w, h): (f64, f64), m: f64, budget: f64, side: f64) -> f64 {
    let (a, b) = (w + 2.0 * m, h + 2.0 * m);
    // The largest `e` with (a + 2e) × (b + 2e) within the budget, less two pixels for the rounding
    // of the raster's edges to whole pixels.
    let e = (((a - b).powi(2) + 4.0 * budget).sqrt() - (a + b)) / 4.0 - 2.0;
    // (Also for a view or spread that isn't a number.)
    if !e.is_finite() {
        return 0.0;
    }
    e.min((side - a.max(b)) / 2.0 - 2.0).min(w.max(h) / 2.0).floor().max(0.0)
}

/// Most pixels a raster of [`Renderer::pixel_content`] or [`Renderer::with_filters`] holds while
/// [`crate::RenderOptions::dragging`] (a 1000 × 1000 raster's), when the frame does not draw a raster
/// kept from an earlier frame instead (#2558).
pub(super) const DRAG_PIXEL_BUDGET: f64 = 1.0e6;

/// The scale (raster pixels a device pixel) for content of `rw` × `rh` device pixels that would
/// render at scale `k`, when a drag lowers it to stay within [`DRAG_PIXEL_BUDGET`]; `None` when it
/// does not (`dragging` off, or content within the budget at `k`).
pub(super) fn drag_scale(dragging: bool, (rw, rh): (f64, f64), k: f64) -> Option<f64> {
    let d = (DRAG_PIXEL_BUDGET / (rw * rh)).sqrt();
    (dragging && d < k).then_some(d)
}

/// The raster a shadow or glow blurred by `blur` device pixels is filtered in, for content
/// reaching `reach` (device pixels) in a view of `screen` pixels: (left, top, width, height,
/// scale). It covers what of the reach can show in the view, the blur's spread around it
/// included, at `scale` raster pixels a device pixel: 1, or less to stay within
/// [`PIXEL_BUDGET`]. At a high zoom the spread of a wide blur is tens of thousands of pixels a
/// side, whatever the size of the view (#1706). `None`: nothing to draw.
pub(super) fn filter_region(reach: Rect, screen: (u16, u16), blur: f64) -> Option<(f64, f64, u16, u16, f64)> {
    filter_region_within(reach, screen, blur, PIXEL_BUDGET)
}

/// The region [`filter_region`] returns, within `budget` pixels instead of [`PIXEL_BUDGET`].
pub(super) fn filter_region_within(reach: Rect, screen: (u16, u16), blur: f64, budget: f64) -> Option<(f64, f64, u16, u16, f64)> {
    // 3σ of the Gaussian (σ = blur / 2) in pixels, plus a pixel of antialiasing.
    let spread = blur.max(0.0) * 1.5 + 2.0;
    let screen = Rect::new(0.0, 0.0, f64::from(screen.0), f64::from(screen.1)).inflate(spread, spread);
    let r = reach.inflate(spread, spread).intersect(screen);
    let (x0, y0) = (r.x0.floor(), r.y0.floor());
    let (rw, rh) = (r.x1.ceil() - x0, r.y1.ceil() - y0);
    // (Also false for a reach or a blur that isn't a number.)
    if !(rw >= 1.0 && rh >= 1.0 && rw.is_finite() && rh.is_finite()) {
        return None;
    }
    let side = f64::from(u16::MAX);
    let k = (budget / (rw * rh)).sqrt().min(side / rw).min(side / rh).min(1.0);
    let (w, h) = ((rw * k).floor().clamp(1.0, side), (rh * k).floor().clamp(1.0, side));
    Some((x0, y0, w as u16, h as u16, k))
}

/// Draw `image` with its top-left at (`x`, `y`) device pixels, `1 / k` device pixels a pixel.
pub(super) fn draw_scaled(ctx: &mut RenderContext, image: std::sync::Arc<vello_cpu::Pixmap>, (x, y): (f64, f64), k: f64) {
    let (w, h) = (image.width() as f64, image.height() as f64);
    ctx.set_transform(Affine::translate((x, y)) * Affine::scale(1.0 / k));
    // On whole pixels at full size the cheapest sampler is exact.
    let quality = if k < 1.0 { peniko::ImageQuality::Medium } else { peniko::ImageQuality::Low };
    let sampler = peniko::ImageSampler { quality, ..Default::default() };
    // The raster maps 1:1 onto the rect: a paint transform left by an earlier fill (a pattern,
    // radial or freeform gradient) would resample it (#1536).
    ctx.reset_paint_transform();
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(image), sampler });
    ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
    ctx.set_transform(Affine::IDENTITY);
}

/// Draw `image` with its pixels placed by `xf` (pixels to device pixels). On whole pixels at full
/// size it is drawn as [`draw_scaled`] draws it.
pub(super) fn draw_placed(ctx: &mut RenderContext, image: std::sync::Arc<vello_cpu::Pixmap>, xf: Affine) {
    let [a, b, c, d, e, f] = xf.as_coeffs();
    let near = |v: f64, to: f64| (v - to).abs() < 1e-6;
    if near(a, 1.0) && near(b, 0.0) && near(c, 0.0) && near(d, 1.0) && near(e, e.round()) && near(f, f.round()) {
        return draw_scaled(ctx, image, (e.round(), f.round()), 1.0);
    }
    let (w, h) = (image.width() as f64, image.height() as f64);
    ctx.set_transform(xf);
    let sampler = peniko::ImageSampler { quality: peniko::ImageQuality::Medium, ..Default::default() };
    ctx.reset_paint_transform();
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(image), sampler });
    ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
    ctx.set_transform(Affine::IDENTITY);
}
