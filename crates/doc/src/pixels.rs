//! Pixel access for embedded images: a stable content key and recolouring the pixels.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use crate::ImageBlob;

/// The most memory the decoded RGBA pixels of one image may take: the allowance the importers
/// and the renderer decode within (a 60 in sheet at 300 ppi is ~475 MB). The web app has 4 GB to
/// address and aborts when an allocation fails, so it keeps the decoder's own 512 MB.
pub const MAX_RASTER_ALLOC: u64 = if cfg!(target_arch = "wasm32") { 512 << 20 } else { 2 << 30 };

/// The pixels of encoded image `bytes` as RGBA; `None` when they can't be decoded, would take
/// more than `max` bytes as RGBA, or need more than `max` bytes to decode. The size is read from
/// the header first: the decoder's limit is on the file's samples, and a gray image is a quarter
/// of what it becomes here (#1717).
fn rgba_within(bytes: &[u8], max: u64) -> Option<image::RgbaImage> {
    let (w, h) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()?;
    if u64::from(w) * u64::from(h) > max / 4 {
        return None;
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max);
    reader.limits(limits);
    Some(reader.decode().ok()?.into_rgba8())
}

impl ImageBlob {
    /// A PNG blob of `png` bytes.
    pub fn png(png: Vec<u8>) -> Self {
        Self::new("image/png", png)
    }

    /// A key derived from the bytes (FNV-1a), the same for identical images.
    pub fn content_key(&self) -> String {
        crate::links::hash_bytes(&self.bytes)
    }

    /// The image with `f` applied to each pixel's (straight) RGB, alpha kept and fully transparent
    /// pixels left alone, re-encoded as PNG. `f` runs once per distinct colour. `None` when the
    /// image can't be decoded or no pixel changed.
    pub fn map_rgb(&self, mut f: impl FnMut([u8; 3]) -> [u8; 3]) -> Option<ImageBlob> {
        let mut img = rgba_within(&self.bytes, MAX_RASTER_ALLOC)?;
        let mut memo: HashMap<[u8; 3], [u8; 3]> = HashMap::new();
        let mut changed = false;
        for px in img.pixels_mut().filter(|p| p.0[3] > 0) {
            let rgb = [px.0[0], px.0[1], px.0[2]];
            let out = *memo.entry(rgb).or_insert_with(|| f(rgb));
            if out != rgb {
                px.0[..3].copy_from_slice(&out);
                changed = true;
            }
        }
        if !changed {
            return None;
        }
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(Self::png(png))
    }
}

/// How a grayscale image's file stores its pixels ([`ImageBlob::gray_source`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraySource {
    /// One bit a pixel: black and white only.
    pub one_bit: bool,
    /// The file has an alpha channel.
    pub alpha: bool,
}

impl ImageBlob {
    /// Whether the image's file stores grayscale or 1-bit pixels (the images a fill color
    /// colorizes), and how; `None` for color images and data that isn't an image. Reads only its
    /// header.
    pub fn gray_source(&self) -> Option<GraySource> {
        let gray = |one_bit: bool, alpha: bool| Some(GraySource { one_bit, alpha });
        if let Some((kind, _)) = crate::cmyk::tiff(&self.bytes) {
            return match kind {
                tiff::ColorType::Gray(bits) => gray(bits == 1, false),
                tiff::ColorType::GrayA(bits) => gray(bits == 1, true),
                _ => None,
            };
        }
        if crate::cmyk::jpeg_components(&self.bytes) == Some(1) {
            return gray(false, false);
        }
        use image::{ExtendedColorType as E, ImageDecoder as _};
        let reader = image::ImageReader::new(Cursor::new(self.bytes.as_slice())).with_guessed_format().ok()?;
        match reader.into_decoder().ok()?.original_color_type() {
            E::L1 => gray(true, false),
            E::La1 => gray(true, true),
            E::L2 | E::L4 | E::L8 | E::L16 => gray(false, false),
            E::La2 | E::La4 | E::La8 | E::La16 => gray(false, true),
            _ => None,
        }
    }

    /// How much of a colorize paint each pixel shows, row by row from the top: how dark the pixel
    /// is (255 less its Rec. 709 luma) scaled by its alpha, so white and transparent pixels are 0.
    /// Returns the width, the height and one byte a pixel; `None` when the image can't be decoded
    /// within [`MAX_RASTER_ALLOC`].
    pub fn coverage(&self) -> Option<(u32, u32, Vec<u8>)> {
        let img = rgba_within(&self.bytes, MAX_RASTER_ALLOC)?;
        let (w, h) = img.dimensions();
        let cover = img
            .pixels()
            .map(|p| {
                let [r, g, b, a] = p.0;
                let luma = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
                ((255.0 - luma).clamp(0.0, 255.0) * a as f32 / 255.0).round() as u8
            })
            .collect();
        Some((w, h, cover))
    }
}

impl ImageBlob {
    /// The colour around pixel (`x`, `y`) (pixel space): the alpha-weighted average of the `size` ×
    /// `size` pixels centred on it, clipped to the image, as straight RGBA. `None` outside the
    /// image, where every pixel is transparent, or when the image can't be decoded. The last image
    /// decoded is kept for the next sample (the Eyedropper samples one image click after click).
    pub fn sample(&self, x: f64, y: f64, size: u32) -> Option<[u8; 4]> {
        sample_pixels(&*self.decoded()?, x, y, size)
    }

    /// [`Self::sample`] at (`x`, `y`) in the pixel space of an image object of `object` (width,
    /// height) pixels, which the decoded pixels may not match (a linked image showing its preview).
    pub fn sample_object(&self, x: f64, y: f64, object: (u32, u32), size: u32) -> Option<[u8; 4]> {
        let img = self.decoded()?;
        let k = |n: u32, of: u32| n as f64 / of.max(1) as f64;
        sample_pixels(&img, x * k(img.width(), object.0), y * k(img.height(), object.1), size)
    }

    /// The colours of a `rows` × `cols` mesh laid over the image: `(rows + 1) * (cols + 1)` straight
    /// RGBA samples, row-major, one per grid line crossing (row 0 is the top). Each is the
    /// alpha-weighted average of the area around the crossing, one grid cell wide and high (a
    /// quarter of each of the neighbouring cells), not a single pixel. A crossing over transparent
    /// pixels gets their average colour at alpha 0. `None` when `rows` or `cols` is 0 or the
    /// image can't be decoded.
    pub fn grid_samples(&self, rows: u32, cols: u32) -> Option<Vec<[u8; 4]>> {
        if rows == 0 || cols == 0 {
            return None;
        }
        let img = self.decoded()?;
        let (w, h) = (img.width() as usize, img.height() as usize);
        if w == 0 || h == 0 {
            return None;
        }
        // The pixel columns [from, to) around grid line `i` of `n`, at least one pixel wide.
        let span = |i: u32, n: u32, len: usize| {
            let at = |k: f64| (k / n as f64 * len as f64).round().clamp(0.0, len as f64) as usize;
            let (from, to) = (at(i as f64 - 0.5), at(i as f64 + 0.5));
            let from = from.min(len - 1);
            (from, to.max(from + 1))
        };
        let mut out = Vec::with_capacity((rows as usize + 1) * (cols as usize + 1));
        for r in 0..=rows {
            let (y0, y1) = span(r, rows, h);
            for c in 0..=cols {
                let (x0, x1) = span(c, cols, w);
                let (mut sum, mut plain, mut alpha, mut n) = ([0u64; 3], [0u64; 3], 0u64, 0u64);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let [pr, pg, pb, pa] = img.get_pixel(x as u32, y as u32).0;
                        let a = pa as u64;
                        for (k, v) in [pr, pg, pb].into_iter().enumerate() {
                            sum[k] += v as u64 * a;
                            plain[k] += v as u64;
                        }
                        alpha += a;
                        n += 1;
                    }
                }
                let n = n.max(1);
                out.push(if alpha == 0 {
                    let m = |k: usize| ((plain[k] + n / 2) / n) as u8;
                    [m(0), m(1), m(2), 0]
                } else {
                    let m = |k: usize| ((sum[k] + alpha / 2) / alpha) as u8;
                    [m(0), m(1), m(2), ((alpha + n / 2) / n) as u8]
                });
            }
        }
        Some(out)
    }

    /// The decoded pixels (the last image decoded is kept for the next sample).
    fn decoded(&self) -> Option<Arc<image::RgbaImage>> {
        thread_local! {
            static LAST: std::cell::RefCell<Option<(String, Arc<image::RgbaImage>)>> = const { std::cell::RefCell::new(None) };
        }
        let key = self.content_key();
        LAST.with_borrow_mut(|last| match last {
            Some((k, img)) if *k == key => Some(img.clone()),
            _ => {
                let img = Arc::new(rgba_within(&self.bytes, MAX_RASTER_ALLOC)?);
                *last = Some((key, img.clone()));
                Some(img)
            }
        })
    }
}

/// [`ImageBlob::sample`] of decoded pixels `img`.
fn sample_pixels(img: &image::RgbaImage, x: f64, y: f64, size: u32) -> Option<[u8; 4]> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (cx, cy) = (x.floor() as i64, y.floor() as i64);
    if !(0..w).contains(&cx) || !(0..h).contains(&cy) {
        return None;
    }
    let r = size.max(1) as i64 / 2;
    let (mut sum, mut alpha, mut n) = ([0u64; 3], 0u64, 0u64);
    for py in (cy - r).max(0)..=(cy + r).min(h - 1) {
        for px in (cx - r).max(0)..=(cx + r).min(w - 1) {
            let [pr, pg, pb, pa] = img.get_pixel(px as u32, py as u32).0;
            let a = pa as u64;
            sum = [sum[0] + pr as u64 * a, sum[1] + pg as u64 * a, sum[2] + pb as u64 * a];
            alpha += a;
            n += 1;
        }
    }
    if alpha == 0 {
        return None;
    }
    let c = |v: u64| ((v + alpha / 2) / alpha) as u8;
    Some([c(sum[0]), c(sum[1]), c(sum[2]), ((alpha + n / 2) / n) as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(px: &[[u8; 4]], w: u32, h: u32) -> ImageBlob {
        let img = image::RgbaImage::from_raw(w, h, px.concat()).unwrap();
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        ImageBlob::png(png)
    }

    fn pixels(b: &ImageBlob) -> Vec<[u8; 4]> {
        image::load_from_memory(&b.bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect()
    }

    /// A GIF whose `w` × `h` screen holds one opaque red pixel at the top left. The file holds no
    /// other pixels, and the decoder makes them transparent.
    fn gif_with_one_red_pixel(w: u16, h: u16) -> Vec<u8> {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&w.to_le_bytes());
        gif.extend_from_slice(&h.to_le_bytes());
        // Flags for a global color table of two colors, the background and aspect bytes, then the
        // table: red and black.
        gif.extend_from_slice(&[0x80, 0, 0, 255, 0, 0, 0, 0, 0]);
        // A 1 × 1 image at (0, 0), its LZW data (clear, color 0, end), and the end of the file.
        gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 0x01, 0, 0x3B]);
        gif
    }

    /// The pixels are decoded within the allowance, not within the decoder's own 512 MB: an RGBA
    /// image of 11,586 × 11,586 pixels (536,941,584 bytes) is recolored and sampled.
    #[test]
    fn pixels_past_the_decoders_own_limit_are_decoded_within_the_allowance() {
        let (w, h) = (11_586, 11_586);
        let rgba = u64::from(w) * u64::from(h) * 4;
        assert!(rgba > 512 << 20);
        let gif = gif_with_one_red_pixel(w, h);
        assert!(rgba_within(&gif, rgba - 1).is_none(), "past the allowance");
        let b = ImageBlob::new("image/gif", gif);
        assert!(b.map_rgb(|[r, g, bl]| [bl, g, r]).is_some(), "red becomes blue");
        assert_eq!(b.sample(0.5, 0.5, 1), Some([255, 0, 0, 255]));
        assert_eq!(b.sample(f64::from(w) - 0.5, f64::from(h) - 0.5, 1), None, "transparent");
    }

    /// #1717: the RGBA size is checked from the header, before anything is decoded: a grey image
    /// the decoder's own limit lets through is four times that as RGBA.
    #[test]
    fn pixels_past_the_allowance_are_not_decoded() {
        let grey = image::GrayImage::from_raw(4, 2, vec![7; 8]).unwrap();
        let mut png = Vec::new();
        grey.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert!(rgba_within(&png, 31).is_none(), "8 grey bytes are 32 as RGBA");
        assert_eq!(rgba_within(&png, 32).map(|i| i.dimensions()), Some((4, 2)));
        assert!(rgba_within(b"not an image", u64::MAX).is_none());
    }

    /// A little-endian TIFF whose header gives `w` × `h` 8-bit gray pixels in one strip. The file holds
    /// no pixel data.
    fn tiff_header(w: u32, h: u32) -> Vec<u8> {
        let mut out = b"II*\0".to_vec();
        out.extend(8u32.to_le_bytes());
        // Each entry is (tag, type: 3 SHORT or 4 LONG, value) and holds one value.
        let entries: [(u16, u16, u32); 9] =
            [(256, 4, w), (257, 4, h), (258, 3, 8), (259, 3, 1), (262, 3, 1), (273, 4, 8), (277, 3, 1), (278, 4, h), (279, 4, 0)];
        out.extend((entries.len() as u16).to_le_bytes());
        for (tag, kind, value) in entries {
            out.extend(tag.to_le_bytes());
            out.extend(kind.to_le_bytes());
            out.extend(1u32.to_le_bytes());
            out.extend(value.to_le_bytes());
        }
        out.extend(0u32.to_le_bytes());
        out
    }

    /// A TIFF's sides go up to 2^32 - 1 pixels: a size whose RGBA bytes don't fit in 64 bits is
    /// refused like any other past the allowance, and an image within the allowance still decodes.
    #[test]
    fn a_size_past_64_bits_as_rgba_is_not_decoded() {
        let huge = tiff_header(1 << 31, 1 << 31);
        assert!(rgba_within(&huge, MAX_RASTER_ALLOC).is_none());
        assert!(rgba_within(&huge, u64::MAX).is_none());
        let gray = image::GrayImage::from_raw(4, 2, vec![7; 8]).unwrap();
        let mut png = Vec::new();
        gray.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert_eq!(rgba_within(&png, 35).map(|i| i.dimensions()), Some((4, 2)), "32 bytes as RGBA, within 35");
    }

    #[test]
    fn samples_a_pixel_or_the_average_around_it() {
        let b = blob(&[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [9, 9, 9, 0]], 4, 1);
        assert_eq!(b.sample(1.5, 0.5, 1), Some([0, 255, 0, 255]));
        assert_eq!(b.sample(1.5, 0.5, 3), Some([85, 85, 85, 255]), "red, green and blue averaged");
        assert_eq!(b.sample(3.2, 0.5, 3), Some([0, 0, 255, 128]), "transparent pixels don't count towards the colour");
        assert_eq!(b.sample(3.2, 0.5, 1), None, "all transparent");
        assert_eq!(b.sample(-0.5, 0.5, 5), None, "outside");
        assert_eq!(b.sample(0.0, 0.0, 5), Some([85, 85, 85, 255]), "clipped to the image");
    }

    #[test]
    fn samples_in_the_pixel_space_of_an_object_with_more_pixels() {
        // An object of 8 × 2 pixels showing these 4 × 1 (a linked image's preview).
        let b = blob(&[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [9, 9, 9, 255]], 4, 1);
        assert_eq!(b.sample_object(5.0, 1.5, (8, 2), 1), Some([0, 0, 255, 255]));
        assert_eq!(b.sample_object(1.0, 0.0, (4, 1), 1), b.sample(1.0, 0.0, 1), "the same size: the same pixels");
    }

    #[test]
    fn maps_each_colour_once_and_keeps_alpha() {
        let b = blob(&[[255, 0, 0, 255], [255, 0, 0, 128], [0, 0, 255, 255], [10, 20, 30, 0]], 2, 2);
        let mut calls = 0;
        let out = b
            .map_rgb(|[r, g, bl]| {
                calls += 1;
                [255 - r, 255 - g, 255 - bl]
            })
            .unwrap();
        assert_eq!(calls, 2, "red and blue, once each; the transparent pixel is skipped");
        assert_eq!(pixels(&out), [[0, 255, 255, 255], [0, 255, 255, 128], [255, 255, 0, 255], [10, 20, 30, 0]]);
        assert_ne!(out.content_key(), b.content_key());
        assert_eq!(b.content_key(), blob(&[[255, 0, 0, 255], [255, 0, 0, 128], [0, 0, 255, 255], [10, 20, 30, 0]], 2, 2).content_key());
        assert!(b.map_rgb(|c| c).is_none(), "nothing changed");
        assert!(ImageBlob::png(vec![1, 2, 3]).map_rgb(|c| c).is_none(), "undecodable");
    }

    /// An uncompressed 1-bit TIFF, BlackIsZero, of `w` × `h` pixels: `rows` holds each row's bits,
    /// padded to a byte, most significant bit first.
    fn bilevel_tiff(w: u16, h: u16, rows: &[u8]) -> Vec<u8> {
        // Tag, type (3 short, 4 long) and value of each entry, in tag order.
        let data_at = 8 + 2 + 9 * 12 + 4;
        let entries: [(u16, u16, u32); 9] = [
            (256, 3, w.into()),
            (257, 3, h.into()),
            (258, 3, 1),
            (259, 3, 1),
            (262, 3, 1),
            (273, 4, data_at),
            (277, 3, 1),
            (278, 3, h.into()),
            (279, 4, rows.len() as u32),
        ];
        let mut out = b"II*\0".to_vec();
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, value) in entries {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(rows);
        out
    }

    fn png_of(img: image::DynamicImage) -> ImageBlob {
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        ImageBlob::png(png)
    }

    #[test]
    fn grayscale_and_1_bit_files_are_told_from_color_ones() {
        let gray = |one_bit, alpha| Some(GraySource { one_bit, alpha });
        let l8 = png_of(image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(2, 1, vec![0, 255]).unwrap()));
        assert_eq!(l8.gray_source(), gray(false, false));
        let la8 = png_of(image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(1, 1, vec![0, 128]).unwrap()));
        assert_eq!(la8.gray_source(), gray(false, true));
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 90).encode(&[0, 255, 0, 255], 2, 2, jpeg_encoder::ColorType::Luma).unwrap();
        assert_eq!(ImageBlob::new("image/jpeg", jpeg).gray_source(), gray(false, false));
        // Black, white; white, black.
        let tif = ImageBlob::new("image/tiff", bilevel_tiff(2, 2, &[0b0100_0000, 0b1000_0000]));
        assert_eq!(tif.gray_source(), gray(true, false));
        assert_eq!(tif.coverage(), Some((2, 2, vec![255, 0, 0, 255])), "black covers, white doesn't");
        // Color images and data that isn't an image.
        assert_eq!(blob(&[[0, 0, 0, 255], [9, 9, 9, 255]], 2, 1).gray_source(), None);
        assert_eq!(ImageBlob::png(vec![1, 2, 3]).gray_source(), None);
    }

    #[test]
    fn coverage_is_darkness_times_alpha() {
        let la8 = png_of(image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(4, 1, vec![0, 255, 255, 255, 51, 255, 0, 128]).unwrap()));
        assert_eq!(la8.coverage(), Some((4, 1, vec![255, 0, 204, 128])));
        assert_eq!(ImageBlob::png(vec![1, 2, 3]).coverage(), None);
    }
}
