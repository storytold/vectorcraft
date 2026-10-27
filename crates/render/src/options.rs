use super::geometry::region_pixels;
use super::*;

/// Largest raster an export may ask [`Renderer::render_region`] for, per side. The CPU rasteriser
/// addresses at most `u16::MAX` pixels per side (and panics at that edge).
pub const MAX_RASTER_SIDE: u32 = 32_768;

/// Largest side of a render context the renderer makes. The CPU rasteriser keeps pixel positions
/// in `u16` and overflows near `u16::MAX` (its tile and bucket arithmetic rounds past it), so
/// contexts stay within the side raster exports are limited to.
pub(crate) const MAX_CONTEXT_SIDE: u32 = MAX_RASTER_SIDE;

/// Largest raster an export may ask for in total (16384², 1 GiB of RGBA pixels).
pub const MAX_RASTER_PIXELS: u64 = 1 << 28;

/// Pixel size of `region` rendered at `scale` (as [`Renderer::render_region`] rounds it), or an error
/// when it exceeds [`MAX_RASTER_SIDE`] / [`MAX_RASTER_PIXELS`]. Raster exports check this first,
/// because the renderer can only clamp (wrong picture) or fail (abort) at those sizes.
pub fn raster_size(region: Rect, scale: f64) -> Result<(u32, u32), String> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(format!("invalid scale {scale}"));
    }
    let (w, h) = region_pixels(region, scale);
    if !(w <= MAX_RASTER_SIDE && h <= MAX_RASTER_SIDE && u64::from(w) * u64::from(h) <= MAX_RASTER_PIXELS) {
        return Err(format!(
            "the image would be {w} × {h} pixels; raster exports are limited to {MAX_RASTER_SIDE} pixels per side and {} megapixels: lower the scale or resolution",
            MAX_RASTER_PIXELS / 1_000_000
        ));
    }
    Ok((w, h))
}

/// Rendering options.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
    /// Outline (wireframe) view: 1 px black paths, no paint.
    pub outline: bool,
    /// Pasteboard colour behind everything (premultiplied RGBA8); `None` = transparent.
    pub background: Option<[u8; 4]>,
    /// Draw artboards as white rectangles (screen view). Export sets this to false.
    pub artboards: bool,
    /// Objects not to draw (e.g. the object being edited by a live drag preview).
    pub hidden: Vec<NodeId>,
    /// Draw the layers as their Layer Options show them on screen: Preview off in outline, Dim
    /// Images fading their images (a template layer's too), a template in Preview in Outline view.
    pub dim_templates: bool,
    /// Soft proof / separations preview (see [`proof`]).
    pub proof: Option<proof::ProofSetup>,
    /// Overprint Preview (see [`proof`] for what overprints).
    pub overprint_preview: bool,
    /// Trim View: clip the artwork to the artboards (nothing on the pasteboard is drawn).
    pub trim: bool,
    /// Leave template layers and guides out (exports and thumbnails: they're aids to drawing, not
    /// artwork).
    pub skip_templates: bool,
    /// View › Hide Template: template layers left out of the screen view (guides stay).
    pub hide_templates: bool,
    /// View › Hide Guides: paths made into guides left out of the screen view. Exports use
    /// `skip_templates`, which independently leaves both templates and guides out.
    pub hide_guides: bool,
    /// Pattern editing mode's tile edge and swatch bounds colour (Object → Pattern → Tile Edge
    /// Color), RGB.
    pub tile_edge: [u8; 3],
    /// Colour and style of the paths made into guides (Preferences › Guides & Grid, as the ruler
    /// guides the UI draws): RGB, and whether they are dotted.
    pub guide: ([u8; 3], bool),
    /// View Opacity Mask (Alt-click the mask thumbnail): instead of the artwork, show this object's
    /// opacity mask alone as its coverage in greyscale (white = opaque, black = transparent).
    pub mask_view: Option<NodeId>,
    /// Screen view: highlight substituted fonts and glyphs, as Preferences › Type and Document
    /// Setup ask.
    pub highlight_substitutions: bool,
    /// Screen view: Preferences › Type › Highlight Substituted Fonts. Pink behind text drawn in a
    /// substitute for a missing font, in every document (on by default).
    pub highlight_substituted_fonts: bool,
    /// Screen view: Preferences › Type › Highlight Alternate Glyphs. Pink behind glyphs OpenType
    /// features substituted (ligatures, contextual forms), not Document Setup's missing-glyph
    /// highlights.
    pub highlight_alternate_glyphs: bool,
    /// Edge smoothing (raster export option).
    pub anti_alias: AntiAlias,
    /// Screen view: placed documents draw from cached bitmaps made in the background (see
    /// [`placed_document`]); off, they draw exactly, read when needed.
    pub progressive_placed: bool,
    /// Screen view: Image Trace objects draw as their View asks (outlines, the source image…);
    /// off, they draw their tracing result, as exports and printing do.
    pub trace_views: bool,
    /// Screen view with `trace_views` on: the Image Trace objects drawn with the Source Image view
    /// for this render only, whatever View they have (the Image Trace panel's eye button, while it
    /// is held down).
    pub trace_source: Option<Arc<Vec<NodeId>>>,
    /// Images are sampled smoothly when scaled or rotated; off, each pixel takes its nearest image
    /// pixel (Pixel Preview with File Handling › Display Bitmaps as Anti-aliased Images off).
    pub smooth_images: bool,
    /// Screen view in isolation mode: the isolated group or layer. Everything around it draws
    /// dimmed ([`ISOLATION_DIM`]).
    pub isolated: Option<NodeId>,
    /// Composite in floating point (exports): translucent art over opaque art comes out exactly
    /// opaque, where 8-bit compositing leaves alpha 254 on some anti-aliased edges (#787). Off,
    /// the faster 8-bit pipeline (the canvas).
    pub precise: bool,
    /// Exports: Art Optimized edges are supersampled ([`supersample_factor`]): the art is drawn
    /// with hard edges at several times the size and averaged down, so shapes that meet edge to
    /// edge leave no seam of the background between them (#983). Off, edges are smoothed shape by
    /// shape (the canvas).
    pub supersample: bool,
    /// Screen views: an image drawn at half its size or smaller samples a copy shrunk by halves
    /// (box-filtered from the full image, kept with the decoded images), so the full image needn't
    /// stay in memory or be sampled. Off, images always sample their full pixels (exports).
    pub image_mips: bool,
    /// Screen views: an image whose pixels can't be decoded draws as a light gray box crossed
    /// from corner to corner. Off, it draws nothing (exports, which report it instead).
    pub mark_undecoded_images: bool,
    /// Screen views while an interaction is in progress (a drag, a dialog's preview): an object
    /// whose content only moved since an earlier frame draws its cached drop shadows, outer glows
    /// and filtered content from the rasters made then, at the nearest whole pixel, without
    /// running the effects again (see `fx`). Off, no raster made that way, or made over one, is
    /// drawn: the effects run again.
    pub interacting: bool,
    /// Screen views during a tool's drag on the canvas: the renderer draws filtered content within
    /// `fx::DRAG_PIXEL_BUDGET` pixels, at a lower resolution where it would need more, unless it
    /// can draw the content from a raster kept from an earlier frame. With [`Renderer::threads`]
    /// above 0 it does the same for drop shadows, outer glows and inner glows. No raster made that
    /// way is kept (see `fx`). Off, they render as in a still frame.
    pub dragging: bool,
    /// Appearance of Black's Rich Black: pure CMYK black (black ink alone at 100%) shows as RGB
    /// black. Off, it shows through the CMYK profile, a dark grey, as other colours do.
    pub rich_black: bool,
}

/// The opacity of the art around an isolated group or layer.
pub const ISOLATION_DIM: f32 = 0.5;

/// How edges are rasterized (raster export option).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AntiAlias {
    /// Hard edges: a pixel is painted when a path, text or image covers at least half of it.
    /// Raster effects (blur, shadows, glows) and pattern tiles stay smooth.
    None,
    /// Smooth edges everywhere.
    #[default]
    Art,
    /// Smooth edges with type snapped to the pixel grid (see `TextLayout::snap_to_pixels`):
    /// crisper small text.
    Type,
}

impl AntiAlias {
    pub const ALL: [Self; 3] = [Self::None, Self::Art, Self::Type];

    /// The id used in command params (`none`, `art`, `type`).
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Art => "art",
            Self::Type => "type",
        }
    }

    /// The mode an id names (any case).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id().eq_ignore_ascii_case(id))
    }

    /// The name shown in options dialogs.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Art => "Art Optimized",
            Self::Type => "Type Optimized",
        }
    }

    /// vello's aliasing threshold: paint a pixel only above half coverage when edges are hard.
    pub(crate) fn threshold(self) -> Option<u8> {
        (self == Self::None).then_some(127)
    }
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            outline: false,
            background: None,
            artboards: false,
            hidden: vec![],
            dim_templates: true,
            proof: None,
            overprint_preview: false,
            trim: false,
            skip_templates: false,
            hide_templates: false,
            hide_guides: false,
            tile_edge: vectorcraft_doc::LAYER_COLORS[0].1,
            guide: ([0x4a, 0xd8, 0xff], false),
            mask_view: None,
            highlight_substitutions: false,
            highlight_substituted_fonts: true,
            highlight_alternate_glyphs: false,
            anti_alias: AntiAlias::Art,
            progressive_placed: false,
            trace_views: false,
            trace_source: None,
            smooth_images: true,
            isolated: None,
            precise: false,
            supersample: false,
            image_mips: false,
            mark_undecoded_images: false,
            interacting: false,
            dragging: false,
            rich_black: false,
        }
    }
}

/// The most pixels a supersampled render draws (the frame and its compositing buffers hold a few
/// times this many bytes).
const MAX_SUPERSAMPLED_PIXELS: u64 = 40_000_000;

/// How many times wider and taller a `w`×`h` render is drawn when supersampled: 4, or less where
/// that would pass [`MAX_SUPERSAMPLED_PIXELS`] or the renderer's 65,535-pixel side; `None` when not
/// even twice fits (the edges are then smoothed shape by shape).
pub fn supersample_factor(w: u32, h: u32) -> Option<u32> {
    (2..=4u32).rev().find(|k| {
        let (sw, sh) = (u64::from(w) * u64::from(*k), u64::from(h) * u64::from(*k));
        sw <= u64::from(MAX_CONTEXT_SIDE) && sh <= u64::from(MAX_CONTEXT_SIDE) && sw * sh <= MAX_SUPERSAMPLED_PIXELS
    })
}

/// Average each `k`×`k` block of `src` (`w`×`h` pixels of 4 premultiplied bytes) into one pixel.
pub(crate) fn downsample(src: &[u8], w: usize, h: usize, k: usize) -> Vec<u8> {
    let (ow, oh) = (w / k.max(1), h / k.max(1));
    let n = (k * k).max(1) as u32;
    let mut out = vec![0u8; ow * oh * 4];
    for (o, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (ox, oy) = (o % ow.max(1), o / ow.max(1));
        let mut acc = [0u32; 4];
        for y in oy * k..(oy + 1) * k {
            let row = y * w + ox * k;
            for p in src.get(row * 4..(row + k) * 4).unwrap_or_default().as_chunks::<4>().0 {
                for (a, v) in acc.iter_mut().zip(p) {
                    *a += u32::from(*v);
                }
            }
        }
        for (d, a) in px.iter_mut().zip(acc) {
            *d = ((a + n / 2) / n).min(255) as u8;
        }
    }
    out
}

/// A rendered image (premultiplied RGBA8, row-major).
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rendered {
    /// Un-premultiplied RGBA8 copy.
    pub fn to_straight(&self) -> Vec<u8> {
        let mut out = self.pixels.clone();
        for px in out.as_chunks_mut::<4>().0 {
            let a = px[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        out
    }
    /// Encode as PNG without metadata (thumbnails, screenshots, embedded rasters; exports use
    /// [`encode`]).
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        let img = image::RgbaImage::from_raw(self.width, self.height, self.to_straight())
            .ok_or("PNG encoding failed: pixel buffer doesn't match the image size")?;
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).map_err(|e| format!("PNG encoding failed: {e}"))?;
        Ok(buf)
    }
    /// Encode as JPEG (flattened on white) at `quality` 1..=100.
    pub fn to_jpeg(&self, quality: u8) -> Result<Vec<u8>, String> {
        encode::jpeg(self, quality, None)
    }
    /// Encode as lossless WebP.
    pub fn to_webp(&self) -> Result<Vec<u8>, String> {
        encode::webp(self)
    }
    /// Straight-alpha RGBA at (x, y).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        let p = &self.pixels[i..i + 4];
        let a = p[3] as u32;
        if a == 0 {
            return [0, 0, 0, 0];
        }
        let un = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
        [un(p[0]), un(p[1]), un(p[2]), p[3]]
    }
}
