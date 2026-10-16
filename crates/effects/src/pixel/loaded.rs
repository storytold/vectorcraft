//! Load Texture (Effect › Distort › Glass): a surface made from an image the user loads.
//!
//! The effect keeps the image's gray levels in its parameters ([`TEXTURE_IMAGE`]: a grayscale PNG,
//! base64), so the document saves the texture with the effect, and copies of the effect carry it.
//! Each pixel's gray is the surface's height there: white stands [`RELIEF`] above black. The image
//! lies one pixel per point at 100 %, centered on the object's center and repeated in both
//! directions.

use std::io::Cursor;
use std::sync::{Arc, Mutex, PoisonError};

use vectorcraft_geom::Point;

use super::texture::luma;

/// The effect parameter that holds a loaded texture: its gray levels as a grayscale PNG, base64.
pub const TEXTURE_IMAGE: &str = "textureImage";

/// The Texture menu's value for the loaded texture.
pub const LOADED: &str = "loaded";

/// How far (points) a loaded texture's white stands above its black: a black pixel beside a white
/// one gives a slope of about 1, the most the built-in surfaces have.
const RELIEF: f64 = 1.0;

/// How many decoded textures [`texture`] keeps.
const KEEP: usize = 4;

/// A loaded texture: `width` × `height` gray levels, row by row from the top.
#[derive(Debug, PartialEq, Eq)]
pub struct LoadedTexture {
    width: u32,
    height: u32,
    gray: Vec<u8>,
}

impl LoadedTexture {
    /// The texture of `width` × `height` straight (not premultiplied) RGBA pixels: each pixel's
    /// luma over middle gray, as much as its alpha covers it (a transparent pixel is middle gray).
    /// `None` when the image is empty or `rgba` doesn't hold exactly that many pixels.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        let n = (width as usize).checked_mul(height as usize).filter(|n| *n > 0)?;
        let (px, rest) = rgba.as_chunks::<4>();
        if px.len() != n || !rest.is_empty() {
            return None;
        }
        let gray = px
            .iter()
            .map(|p| {
                let a = f32::from(p[3]) / 255.0;
                let l = luma([p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0));
                ((a * l + (1.0 - a) * 0.5) * 255.0).round().clamp(0.0, 255.0) as u8
            })
            .collect();
        Some(Self { width, height, gray })
    }

    /// The size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Its [`TEXTURE_IMAGE`] value: the gray levels as a grayscale PNG, base64; `None` when the
    /// PNG can't be written.
    pub fn to_stored(&self) -> Option<String> {
        let img = image::GrayImage::from_raw(self.width, self.height, self.gray.clone())?;
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(base64_encode(&png))
    }

    /// The texture of a PNG's gray levels, decoded within [`vectorcraft_doc::MAX_RASTER_ALLOC`];
    /// `None` when `png` isn't a PNG or holds no pixels.
    fn from_png(png: &[u8]) -> Option<Self> {
        let mut reader = image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(vectorcraft_doc::MAX_RASTER_ALLOC);
        reader.limits(limits);
        let img = reader.decode().ok()?.to_luma8();
        let (width, height) = img.dimensions();
        (width > 0 && height > 0).then(|| Self { width, height, gray: img.into_raw() })
    }

    /// The surface's height (points) at `p` (points at 100 %, from the object's center): the gray
    /// levels around it interpolated, 0 for black and [`RELIEF`] for white.
    pub(super) fn height(&self, p: Point) -> f64 {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        // Pixel centers lie at whole coordinates, and the image's center lies at the origin.
        let (x, y) = (p.x + 0.5 * w - 0.5, p.y + 0.5 * h - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f64, j: f64| {
            let (i, j) = (i.rem_euclid(w) as usize, j.rem_euclid(h) as usize);
            self.gray.get(j * self.width as usize + i).map_or(0.5, |g| f64::from(*g) / 255.0)
        };
        let top = at(x0, y0) + (at(x0 + 1.0, y0) - at(x0, y0)) * fx;
        let bottom = at(x0, y0 + 1.0) + (at(x0 + 1.0, y0 + 1.0) - at(x0, y0 + 1.0)) * fx;
        RELIEF * (top + (bottom - top) * fy)
    }
}

/// The textures [`texture`] decoded last, with their stored form, the newest last.
static DECODED: Mutex<Vec<(String, Arc<LoadedTexture>)>> = Mutex::new(Vec::new());

/// The texture a [`TEXTURE_IMAGE`] value holds; `None` when it isn't base64 of a PNG. The last
/// [`KEEP`] textures decoded are kept for the next call.
pub fn texture(stored: &str) -> Option<Arc<LoadedTexture>> {
    let mut decoded = DECODED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(i) = decoded.iter().position(|(s, _)| s == stored) {
        let entry = decoded.remove(i);
        let t = entry.1.clone();
        decoded.push(entry);
        return Some(t);
    }
    let t = Arc::new(LoadedTexture::from_png(&base64_decode(stored)?)?);
    if decoded.len() >= KEEP {
        decoded.remove(0);
    }
    decoded.push((stored.to_string(), t.clone()));
    Some(t)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (RFC 4648, with padding), written as `vectorcraft_format::base64_encode`
/// writes it.
fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Standard base64 (RFC 4648, with padding; whitespace skipped), read as
/// `vectorcraft_format::base64_decode` reads it; `None` when it isn't base64.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| B64.iter().position(|&b| b == c).map(|p| p as u32);
    let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for c in clean.chunks(4) {
        let mut n = 0u32;
        let mut pad = 0;
        for &b in c {
            n <<= 6;
            if b == b'=' {
                pad += 1;
            } else {
                n |= val(b)?;
            }
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}
