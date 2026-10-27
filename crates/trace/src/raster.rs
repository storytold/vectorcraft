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
        let decode_err = |e: image::ImageError| TraceError::Decode(e.to_string());
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| TraceError::Decode(e.to_string()))?;
        let (w, h) = reader.into_dimensions().map_err(decode_err)?;
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        if u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(TraceError::TooLarge { width: w, height: h, max: MAX_PIXELS >> 20 });
        }
        let img = image::load_from_memory(bytes).map_err(decode_err)?.to_rgba8();
        let (w, h) = img.dimensions();
        Ok(Self { width: w, height: h, rgba: img.into_raw() })
    }
    /// Decode as [`Raster::decode`], but an image of more than [`MAX_PIXELS`] (up to
    /// [`MAX_SOURCE_PIXELS`]) is scaled down, aspect kept, to the largest size within
    /// [`MAX_PIXELS`] instead of being refused: a 300 ppi scan of a poster traces at slightly
    /// less than its own resolution. The caller maps the result back with `width` / `height`.
    pub fn decode_to_fit(bytes: &[u8]) -> Result<Self, TraceError> {
        let decode_err = |e: image::ImageError| TraceError::Decode(e.to_string());
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| TraceError::Decode(e.to_string()))?;
        let (w, h) = reader.into_dimensions().map_err(decode_err)?;
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        let pixels = u64::from(w) * u64::from(h);
        if pixels <= MAX_PIXELS {
            return Self::decode(bytes);
        }
        if pixels > MAX_SOURCE_PIXELS {
            return Err(TraceError::TooLarge { width: w, height: h, max: MAX_SOURCE_PIXELS >> 20 });
        }
        // Decoded within the allocation the source limit needs; the web keeps file import's 512 MB.
        // A deeper image (16 bits a channel) that needs more fails with a decode error.
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| TraceError::Decode(e.to_string()))?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(if cfg!(target_arch = "wasm32") { 512 << 20 } else { MAX_SOURCE_PIXELS * 4 });
        reader.limits(limits);
        let img = reader.decode().map_err(decode_err)?.into_rgba8();
        let (w, h) = img.dimensions();
        let (nw, nh) = fit_size(w, h, MAX_PIXELS);
        // Box-filtered: every source pixel counts, so thin lines and anti-aliased edges survive.
        let small = image::imageops::thumbnail(&img, nw, nh);
        Ok(Self { width: nw, height: nh, rgba: small.into_raw() })
    }
    /// Encode as PNG.
    pub fn encode_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(img) = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone()) {
            let _ = img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
        }
        out
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
