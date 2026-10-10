//! Raster export: render a document region with [`RasterExportOptions`] (resolution, background,
//! anti-aliasing) and encode it as PNG ([`png`]: resolution in `pHYs`, Adam7 interlacing), JPEG
//! ([`jpeg`]: RGB, CMYK or grey, progressive, resolution and colour profile), lossless WebP, or
//! a palette image ([`quantize`]) as PNG-8 or [`gif`], [`tiff`] (RGB, CMYK or grey, LZW, either byte
//! order, the profile), [`bmp`] (1–32 bits, RLE) or [`tga`]; [`web`] adds what Save for Web optimises
//! further (web snap, colour table edits, lossy GIF, comments); [`psd`] writes layered bitmaps.

pub mod bmp;
pub mod gif;
pub mod jpeg;
pub mod png;
pub mod psd;
pub mod quantize;
pub mod tga;
pub mod tiff;
pub mod web;

use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::Rect;

use crate::{AntiAlias, RenderOptions, Rendered, Renderer, fx};

/// A raster file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterFormat {
    Png,
    Jpeg,
    WebP,
    /// Indexed PNG: a palette of up to 256 colours.
    Png8,
    Gif,
    Tiff,
    Bmp,
    Tga,
    /// Layered bitmap (.psd).
    Psd,
}

/// How a raster export renders and encodes.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterExportOptions {
    /// Resolution in pixels per inch: 72 renders one pixel per point. Stored in the file.
    pub ppi: f64,
    /// Opaque background colour (RGB); `None` = transparent (JPEG has no alpha: white).
    pub background: Option<[u8; 3]>,
    pub anti_alias: AntiAlias,
    /// PNG, PNG-8: Adam7 interlacing; GIF: interlaced rows.
    pub interlaced: bool,
    /// JPEG quality 0–100.
    pub quality: u8,
    /// JPEG colour model, coding and profile.
    pub jpeg: jpeg::JpegOptions,
    /// PNG-8 and GIF: the colour reduction (also BMP's at 1, 4 and 8 bits).
    pub palette: quantize::PaletteOptions,
    /// TIFF colour model, compression, byte order and profile.
    pub tiff: tiff::TiffOptions,
    /// BMP layout, depth, compression and row order.
    pub bmp: bmp::BmpOptions,
    /// Targa depth.
    pub tga: tga::TgaOptions,
    /// PSD colour model, layers and profile.
    pub psd: psd::PsdOptions,
}

impl Default for RasterExportOptions {
    fn default() -> Self {
        Self {
            ppi: 72.0,
            background: None,
            anti_alias: AntiAlias::Art,
            interlaced: false,
            quality: 90,
            jpeg: jpeg::JpegOptions::default(),
            palette: quantize::PaletteOptions::default(),
            tiff: tiff::TiffOptions::default(),
            bmp: bmp::BmpOptions::default(),
            tga: tga::TgaOptions::default(),
            psd: psd::PsdOptions::default(),
        }
    }
}

impl RasterExportOptions {
    /// Pixels per point.
    pub fn scale(&self) -> f64 {
        self.ppi / 72.0
    }

    /// What the renderer draws for `format`: template layers and guides left out, over the background
    /// (white for a JPEG or a flat PSD without one), composited in floating point.
    pub fn render_options(&self, format: RasterFormat) -> RenderOptions {
        let flat = format == RasterFormat::Jpeg || (format == RasterFormat::Psd && !self.psd.layers);
        let background = self.background.or(flat.then_some([255; 3]));
        RenderOptions {
            background: background.map(|[r, g, b]| [r, g, b, 255]),
            skip_templates: true,
            anti_alias: self.anti_alias,
            // Opaque art stays exactly opaque where translucent edges cross it (#787).
            precise: true,
            ..Default::default()
        }
    }

    /// Encode a rendered image as `format`. A CMYK JPEG separates the screen colours, and a PSD is
    /// flat; [`Renderer::export_region`] draws the inks and the layers instead.
    pub fn encode(&self, img: &Rendered, format: RasterFormat) -> Result<Vec<u8>, String> {
        match format {
            RasterFormat::Png => {
                png::encode(&img.to_straight(), img.width, img.height, &png::PngOptions { ppi: Some(self.ppi), interlaced: self.interlaced })
            }
            RasterFormat::Jpeg => {
                let px = match self.jpeg.color_model {
                    jpeg::ColorModel::Cmyk => jpeg::separated(img),
                    model => jpeg::screen_pixels(img, model == jpeg::ColorModel::Gray),
                };
                jpeg::encode(&px, img.width, img.height, self.quality, Some(self.ppi), &self.jpeg)
            }
            RasterFormat::WebP => webp(img),
            RasterFormat::Png8 | RasterFormat::Gif => {
                let ix = quantize::quantize(&img.to_straight(), img.width, img.height, &self.palette);
                match format {
                    RasterFormat::Gif => gif::encode(&ix, self.interlaced),
                    _ => png::encode_indexed(&ix, &png::PngOptions { ppi: Some(self.ppi), interlaced: self.interlaced }),
                }
            }
            RasterFormat::Tiff => tiff::encode(img, self.ppi, &self.tiff),
            RasterFormat::Bmp => bmp::encode(&img.to_straight(), img.width, img.height, self.ppi, &self.bmp, &self.palette),
            RasterFormat::Tga => tga::encode(&img.to_straight(), img.width, img.height, &self.tga),
            RasterFormat::Psd => psd::encode(img, self.ppi, &self.psd),
        }
    }

    /// Does `format` write ink amounts (a CMYK JPEG or TIFF)?
    fn cmyk(&self, format: RasterFormat) -> bool {
        match format {
            RasterFormat::Jpeg => self.jpeg.color_model == jpeg::ColorModel::Cmyk,
            RasterFormat::Tiff => self.tiff.color_model == jpeg::ColorModel::Cmyk,
            _ => false,
        }
    }
}

impl Renderer {
    /// Render `region` of `doc` (an artboard or any rect) as exported and encode it as `format`.
    /// Callers check the size first ([`crate::raster_size`]). A CMYK JPEG or TIFF is drawn as ink
    /// amounts ([`Renderer::render_region_inks`]); a PSD with its layers ([`psd::export`]).
    pub fn export_region(&mut self, doc: &Document, region: Rect, format: RasterFormat, opts: &RasterExportOptions) -> Result<Vec<u8>, String> {
        if format == RasterFormat::Psd {
            return psd::export(self, doc, region, opts);
        }
        if opts.cmyk(format) {
            let (w, h) = crate::region_pixels(region, opts.scale());
            let inks = self.render_region_inks(doc, region, opts.scale(), &opts.render_options(format));
            return match format {
                RasterFormat::Tiff => tiff::encode_samples(&inks, w, h, opts.ppi, &opts.tiff),
                _ => jpeg::encode(&inks, w, h, opts.quality, Some(opts.ppi), &opts.jpeg),
            };
        }
        let img = self.render_region_with(doc, region, opts.scale(), &opts.render_options(format));
        opts.encode(&img, format)
    }
}

/// Baseline RGB JPEG at `quality` 1–100, partly transparent pixels flattened on white, with the
/// resolution `ppi` (if any) in the JFIF header and no colour profile.
pub(crate) fn jpeg(img: &Rendered, quality: u8, ppi: Option<f64>) -> Result<Vec<u8>, String> {
    let o = jpeg::JpegOptions { embed_icc: false, ..Default::default() };
    jpeg::encode(&jpeg::screen_pixels(img, false), img.width, img.height, quality, ppi, &o)
}

/// A straight-alpha pixel composited over white.
pub fn on_white(p: &[u8; 4]) -> [u8; 3] {
    let a = u32::from(p[3]);
    [0, 1, 2].map(|i| ((u32::from(p[i]) * a + 255 * (255 - a) + 127) / 255) as u8)
}

/// Lossless WebP.
pub(crate) fn webp(img: &Rendered) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut buf);
    image::ImageEncoder::write_image(enc, &img.to_straight(), img.width, img.height, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("WebP encoding failed: {e}"))?;
    Ok(buf)
}

/// Bounds of the art an export draws: visible objects off template layers (guides left out),
/// with their strokes and live effects. `None` when nothing would be drawn.
pub fn art_bounds(doc: &Document) -> Option<Rect> {
    doc.layers.iter().fold(None, |acc, l| vectorcraft_geom::union_opt(acc, drawn_bounds(l)))
}

fn drawn_bounds(n: &Node) -> Option<Rect> {
    if !n.visible {
        return None;
    }
    match &n.kind {
        NodeKind::Layer { template: true, .. } | NodeKind::Path { guide: true, .. } => None,
        NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
            children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, drawn_bounds(c)))
        }
        _ if fx::has_fx(n) => fx::visual_bounds(n),
        _ => n.visual_bounds(),
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_bmp;
#[cfg(test)]
mod tests_jpeg;
#[cfg(test)]
mod tests_palette;
#[cfg(test)]
mod tests_psd;
#[cfg(test)]
mod tests_tga;
#[cfg(test)]
mod tests_tiff;
#[cfg(test)]
mod tests_web;
