//! Rendering helpers and image comparison with a perceptual tolerance.

use vectorcraft_doc::Document;
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_render::{RenderOptions, Rendered, Renderer};

/// A straight-alpha RGBA8 image.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn from_rendered(r: &Rendered) -> Self {
        Self { width: r.width, height: r.height, rgba: r.to_straight() }
    }
    /// Decode a PNG.
    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        let img = craft_image_codecs::decode_as(craft_image_codecs::Format::Png, bytes).map_err(|e| e.to_string())?;
        Ok(Self { width: img.width(), height: img.height(), rgba: img.to_rgba8() })
    }
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
    /// The pixel composited over opaque white (what a viewer sees).
    pub fn over_white(&self, x: u32, y: u32) -> [u8; 3] {
        let [r, g, b, a] = self.pixel(x, y);
        let f = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32) + 127) / 255) as u8;
        [f(r), f(g), f(b)]
    }
    /// Number of pixels for which `pred(rgba)` holds.
    pub fn count(&self, pred: impl Fn([u8; 4]) -> bool) -> usize {
        self.rgba.as_chunks::<4>().0.iter().filter(|p| pred([p[0], p[1], p[2], p[3]])).count()
    }
    /// Pixels that differ from opaque white and are not fully transparent.
    pub fn ink(&self) -> usize {
        self.count(|p| p[3] > 0 && !(p[0] > 250 && p[1] > 250 && p[2] > 250))
    }
    /// Save as PNG (for debugging failed comparisons).
    pub fn save_png(&self, path: &std::path::Path) {
        if let Ok(img) = craft_image_codecs::Image::from_raw(
            self.width,
            self.height,
            craft_image_codecs::ChannelLayout::Rgba,
            craft_image_codecs::SampleType::U8,
            self.rgba.clone(),
        ) && let Ok(png) = craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
        {
            let _ = std::fs::write(path, png);
        }
    }
    /// Luminance over white, 0..255 as f64, per pixel.
    fn luma_over_white(&self) -> Vec<[f64; 3]> {
        (0..self.height).flat_map(|y| (0..self.width).map(move |x| (x, y))).map(|(x, y)| self.over_white(x, y).map(|c| c as f64)).collect()
    }
}

/// Render `doc` at `scale` px/pt over the region `r` on a white background.
pub fn render_region(doc: &Document, r: Rect, scale: f64) -> Image {
    Image::from_rendered(&Renderer::new().render_region(doc, r, scale, true))
}

/// Render the first artboard at 1 px/pt on white.
pub fn render_artboard(doc: &Document) -> Image {
    render_region(doc, doc.artboards[0].rect, 1.0)
}

/// Render `w`×`h` pixels with an identity view (doc points = pixels) on white.
pub fn render_view(doc: &Document, w: u32, h: u32) -> Image {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    Image::from_rendered(&Renderer::new().render(doc, w, h, Affine::IDENTITY, &opts))
}

/// Max per-channel difference between two pixels.
pub fn channel_diff(a: [u8; 4], b: [u8; 4]) -> u8 {
    (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap_or(0)
}

/// Assert a pixel is within `tol` (per channel) of `want`.
#[track_caller]
pub fn assert_pixel(img: &Image, x: u32, y: u32, want: [u8; 4], tol: u8) {
    let got = img.pixel(x, y);
    assert!(channel_diff(got, want) <= tol, "pixel ({x},{y}) = {got:?}, want {want:?} ±{tol}");
}

/// Assert a pixel, composited over white, is within `tol` of `want` RGB.
#[track_caller]
pub fn assert_rgb(img: &Image, x: u32, y: u32, want: [u8; 3], tol: u8) {
    let got = img.over_white(x, y);
    let d = (0..3).map(|i| got[i].abs_diff(want[i])).max().unwrap();
    assert!(d <= tol, "pixel ({x},{y}) = {got:?}, want {want:?} ±{tol}");
}

/// Statistics from comparing two same-sized images.
#[derive(Clone, Copy, Debug, Default)]
pub struct DiffStats {
    /// Largest per-channel difference over white.
    pub max: u8,
    /// Mean absolute difference over all channels (0..255).
    pub mean: f64,
    /// Pixels whose (blurred, perceptual) difference exceeds the tolerance.
    pub bad_pixels: usize,
    pub pixels: usize,
}

/// Exact per-pixel comparison (composited over white).
pub fn diff(a: &Image, b: &Image) -> DiffStats {
    assert_eq!((a.width, a.height), (b.width, b.height), "image sizes differ");
    let (la, lb) = (a.luma_over_white(), b.luma_over_white());
    let mut st = DiffStats { pixels: la.len(), ..Default::default() };
    let mut sum = 0.0;
    for (p, q) in la.iter().zip(&lb) {
        for c in 0..3 {
            let d = (p[c] - q[c]).abs();
            sum += d;
            st.max = st.max.max(d as u8);
        }
    }
    st.mean = sum / (la.len() * 3).max(1) as f64;
    st
}

/// Perceptual comparison: both images are box-blurred (radius 1) to forgive sub-pixel
/// anti-aliasing shifts, then compared with a luminance-weighted colour distance. A pixel is "bad"
/// when that distance exceeds `tol` (0..255 scale).
pub fn perceptual_diff(a: &Image, b: &Image, tol: f64) -> DiffStats {
    assert_eq!((a.width, a.height), (b.width, b.height), "image sizes differ");
    let (w, h) = (a.width as i64, a.height as i64);
    let blur = |img: &Image| {
        let l = img.luma_over_white();
        let mut out = vec![[0.0; 3]; l.len()];
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0; 3];
                let mut n = 0.0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (xx, yy) = (x + dx, y + dy);
                        if xx >= 0 && yy >= 0 && xx < w && yy < h {
                            let p = l[(yy * w + xx) as usize];
                            for c in 0..3 {
                                acc[c] += p[c];
                            }
                            n += 1.0;
                        }
                    }
                }
                out[(y * w + x) as usize] = acc.map(|v| v / n);
            }
        }
        out
    };
    let (ba, bb) = (blur(a), blur(b));
    let mut st = diff(a, b);
    st.bad_pixels = 0;
    for (p, q) in ba.iter().zip(&bb) {
        // Rec. 601 luma weights for the perceptual distance; chroma counts at half weight.
        let dy = 0.299 * (p[0] - q[0]) + 0.587 * (p[1] - q[1]) + 0.114 * (p[2] - q[2]);
        let dc = ((p[0] - q[0]).abs() + (p[1] - q[1]).abs() + (p[2] - q[2]).abs()) / 3.0;
        if dy.abs().max(dc * 0.5) > tol {
            st.bad_pixels += 1;
        }
    }
    st
}

/// Assert two images are perceptually equal: at most `max_bad_fraction` of pixels exceed `tol`.
#[track_caller]
pub fn assert_similar(a: &Image, b: &Image, tol: f64, max_bad_fraction: f64) {
    let st = perceptual_diff(a, b, tol);
    let frac = st.bad_pixels as f64 / st.pixels.max(1) as f64;
    if frac > max_bad_fraction {
        let dir = crate::temp_dir("raster");
        a.save_png(&dir.join("a.png"));
        b.save_png(&dir.join("b.png"));
        panic!("images differ: {st:?} ({:.3}% bad > {:.3}%), saved to {}", frac * 100.0, max_bad_fraction * 100.0, dir.display());
    }
}
