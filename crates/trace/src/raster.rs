/// Errors decoding a raster.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("could not decode image: {0}")]
    Decode(String),
    #[error("image is empty")]
    Empty,
    #[error("the image is {width} × {height} pixels, more than Image Trace takes ({max} megapixels)")]
    TooLarge { width: u32, height: u32, max: u64 },
    #[error("this isn't a flat-colour logo ({:.0}% of its solid areas match a few flat colours), so Flat Logo mode would turn its gradients into patches: use Color mode instead", (.fraction * 100.0).floor())]
    NotFlat { fraction: f64 },
    #[error("\"{0}\" is not a colour: write it as #rrggbb")]
    BadPalette(String),
    #[error(
        "this trace would make more than {max} anchor points, more than the document can show smoothly: raise Noise, lower Paths or Colors, or pick a lower-fidelity preset"
    )]
    TooComplex { max: usize },
    /// The caller of [`crate::trace_watched`] stopped the trace.
    #[error("the trace was cancelled")]
    Cancelled,
}

/// The most pixels an image to trace may have (64 megapixels: 256 MB decoded); larger images are
/// refused instead of running out of memory.
pub const MAX_PIXELS: u64 = 64 << 20;

/// The most pixels an image may have for [`Raster::decode_to_fit`] to decode it and scale it down
/// to [`MAX_PIXELS`] (192 megapixels: 768 MB decoded); larger images are refused.
pub const MAX_SOURCE_PIXELS: u64 = 192 << 20;

/// The largest size with `w : h`'s aspect of at most `max` pixels (each side at least 1).
pub(crate) fn fit_size(w: u32, h: u32, max: u64) -> (u32, u32) {
    let pixels = u64::from(w) * u64::from(h);
    if pixels <= max || pixels == 0 {
        return (w, h);
    }
    let k = (max as f64 / pixels as f64).sqrt();
    let side = |v: u32| ((f64::from(v) * k).floor() as u32).clamp(1, v);
    let (mut nw, mut nh) = (side(w), side(h));
    // A side held at 1 pixel leaves the other too long: cut it to what fits.
    if u64::from(nw) * u64::from(nh) > max {
        if nw >= nh {
            nw = u32::try_from(max / u64::from(nh)).unwrap_or(nw).clamp(1, nw);
        } else {
            nh = u32::try_from(max / u64::from(nw)).unwrap_or(nh).clamp(1, nh);
        }
    }
    (nw, nh)
}

/// An RGBA8 raster (row-major, top row first).
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The pixels the first frame of GIF `bytes` declares in its image descriptor, which the decoder
/// allocates for before it crops the frame to the logical screen the dimension check reads. `None`
/// when the bytes aren't a GIF or hold no frame header.
pub(crate) fn gif_first_frame_pixels(bytes: &[u8]) -> Option<u64> {
    if !bytes.starts_with(b"GIF8") {
        return None;
    }
    let flags = *bytes.get(10)?;
    let mut at = 13 + if flags & 0x80 != 0 { 3 << ((flags & 7) + 1) } else { 0 };
    loop {
        match *bytes.get(at)? {
            // An extension: its data sub-blocks end at a zero length.
            0x21 => {
                at += 2;
                loop {
                    let len = usize::from(*bytes.get(at)?);
                    at += 1 + len;
                    if len == 0 {
                        break;
                    }
                }
            }
            0x2C => {
                let d = bytes.get(at + 5..at + 9)?;
                return Some(u64::from(u16::from_le_bytes([d[0], d[1]])) * u64::from(u16::from_le_bytes([d[2], d[3]])));
            }
            _ => return None,
        }
    }
}

impl Raster {
    /// A raster from RGBA bytes, which must be exactly `width * height * 4` of them: any other
    /// length (bytes from a file, a plug-in or a message) is a [`TraceError::Decode`] error, not
    /// a crash. (That error's text is already translated; its detail names the sizes.)
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, TraceError> {
        let want = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4));
        if want != Some(rgba.len()) {
            return Err(TraceError::Decode(format!("{} bytes of RGBA data for a {width} × {height} image", rgba.len())));
        }
        Ok(Self { width, height, rgba })
    }
    /// A raster whose pixel `(x, y)` is `f(x, y)`.
    pub fn from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Self {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Self { width, height, rgba }
    }
    /// Decode PNG / JPEG / WebP / GIF bytes (at most [`MAX_PIXELS`], checked before decoding).
    pub fn decode(bytes: &[u8]) -> Result<Self, TraceError> {
        if let Some(info) = craft_image_codecs::inspect_raster(bytes).map_err(|e| TraceError::Decode(e.to_string()))? {
            if info.width == 0 || info.height == 0 {
                return Err(TraceError::Empty);
            }
            if u64::from(info.width) * u64::from(info.height) > MAX_PIXELS
                || gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_PIXELS)
            {
                return Err(TraceError::TooLarge { width: info.width, height: info.height, max: MAX_PIXELS >> 20 });
            }
        } else if gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_PIXELS) {
            return Err(TraceError::TooLarge { width: 0, height: 0, max: MAX_PIXELS >> 20 });
        }
        let opts = craft_image_codecs::DecodeOptions {
            limits: craft_image_codecs::Limits {
                max_pixels: MAX_PIXELS,
                ..craft_image_codecs::Limits::default()
            },
            ..craft_image_codecs::DecodeOptions::default()
        };
        let img = craft_image_codecs::decode_with(bytes, &opts).map_err(|e| TraceError::Decode(e.to_string()))?;
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        Ok(Self { width: w, height: h, rgba: img.to_rgba8() })
    }
    /// Decode as [`Raster::decode`], but an image of more than [`MAX_PIXELS`] (up to
    /// [`MAX_SOURCE_PIXELS`]) is scaled down, aspect kept, to the largest size within
    /// [`MAX_PIXELS`] instead of being refused: a 300 ppi scan of a poster traces at slightly
    /// less than its own resolution. The caller maps the result back with `width` / `height`.
    pub fn decode_to_fit(bytes: &[u8]) -> Result<Self, TraceError> {
        if let Some(info) = craft_image_codecs::inspect_raster(bytes).map_err(|e| TraceError::Decode(e.to_string()))? {
            if info.width == 0 || info.height == 0 {
                return Err(TraceError::Empty);
            }
            let pixels = u64::from(info.width) * u64::from(info.height);
            if pixels <= MAX_PIXELS && !gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_PIXELS) {
                return Self::decode(bytes);
            }
            if pixels > MAX_SOURCE_PIXELS {
                return Err(TraceError::TooLarge {
                    width: info.width,
                    height: info.height,
                    max: MAX_SOURCE_PIXELS >> 20,
                });
            }
            if gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_SOURCE_PIXELS) {
                return Err(TraceError::TooLarge {
                    width: info.width,
                    height: info.height,
                    max: MAX_SOURCE_PIXELS >> 20,
                });
            }
        } else if gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_SOURCE_PIXELS) {
            return Err(TraceError::TooLarge { width: 0, height: 0, max: MAX_SOURCE_PIXELS >> 20 });
        }
        // Decoded within the allocation the source limit needs; the web keeps file import's 512 MB.
        // A deeper image (16 bits a channel) that needs more fails with a decode error.
        let opts = craft_image_codecs::DecodeOptions {
            limits: craft_image_codecs::Limits {
                max_pixels: MAX_SOURCE_PIXELS,
                max_alloc: if cfg!(target_arch = "wasm32") { 512 << 20 } else { MAX_SOURCE_PIXELS * 4 },
                ..craft_image_codecs::Limits::default()
            },
            ..craft_image_codecs::DecodeOptions::default()
        };
        let img = craft_image_codecs::decode_with(bytes, &opts).map_err(|e| TraceError::Decode(e.to_string()))?;
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        let rgba = img.to_rgba8();
        if u64::from(w) * u64::from(h) <= MAX_PIXELS {
            return Ok(Self { width: w, height: h, rgba });
        }
        let (nw, nh) = fit_size(w, h, MAX_PIXELS);
        // Box-filtered: every source pixel counts, so thin lines and anti-aliased edges survive.
        let small = box_thumbnail(&rgba, w, h, nw, nh);
        Ok(Self { width: nw, height: nh, rgba: small })
    }
    /// Encode as PNG.
    pub fn encode_png(&self) -> Vec<u8> {
        let Ok(img) = craft_image_codecs::Image::from_u8(
            self.width,
            self.height,
            craft_image_codecs::ChannelLayout::Rgba,
            self.rgba.clone(),
        ) else {
            return Vec::new();
        };
        craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &Default::default()).unwrap_or_default()
    }
    /// Pixel `(x, y)`; transparent black outside the raster (or past the end of a raster whose
    /// fields were set to a size its bytes don't have).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        match self.rgba.get(i..i + 4) {
            Some(&[r, g, b, a]) => [r, g, b, a],
            _ => [0; 4],
        }
    }
}

/// Box-filter downscale of an RGBA8 buffer from `sw × sh` to `dw × dh`.
fn box_thumbnail(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(dw as usize * dh as usize * 4);
    for dy in 0..dh {
        let y0 = (u64::from(dy) * u64::from(sh) / u64::from(dh)) as u32;
        let y1 = ((u64::from(dy) + 1) * u64::from(sh)).div_ceil(u64::from(dh)).clamp(u64::from(y0) + 1, u64::from(sh)) as u32;
        for dx in 0..dw {
            let x0 = (u64::from(dx) * u64::from(sw) / u64::from(dw)) as u32;
            let x1 = ((u64::from(dx) + 1) * u64::from(sw)).div_ceil(u64::from(dw)).clamp(u64::from(x0) + 1, u64::from(sw)) as u32;
            let (mut r, mut g, mut b, mut a, mut count) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                let row = y as usize * sw as usize * 4;
                for x in x0..x1 {
                    let i = row + x as usize * 4;
                    if let Some(&[pr, pg, pb, pa]) = src.get(i..i + 4) {
                        r += u64::from(pr);
                        g += u64::from(pg);
                        b += u64::from(pb);
                        a += u64::from(pa);
                        count += 1;
                    }
                }
            }
            let half = count / 2;
            let div = |sum: u64| if count == 0 { 0 } else { ((sum + half) / count) as u8 };
            out.extend_from_slice(&[div(r), div(g), div(b), div(a)]);
        }
    }
    out
}
