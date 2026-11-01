//! Device-independent bitmaps (DIBs), the pixels inside image records: written bottom-up as 24-bit
//! (opaque, composited over white) or 32-bit premultiplied BGRA (AlphaBlend's source); read from 1,
//! 4, 8, 16, 24 and 32-bit uncompressed and bit-field bitmaps and from embedded JPEG and PNG.

use std::io::Cursor;

use crate::bytes::Reader;

/// Most pixels along a side of an image the writers store (larger ones are scaled down).
const MAX_SIDE: u32 = 8192;
/// Most pixels an image the writers store holds.
const MAX_PIXELS: u64 = 1 << 24;
/// Most pixels along a side, and in all, of a bitmap the reader decodes.
const MAX_READ_SIDE: u32 = 1 << 15;
const MAX_READ_PIXELS: u64 = 1 << 26;
/// Most pixels the bitmaps of one file may decode to together (four of the largest one), so a small
/// file of many image records can't cost minutes of decoding (#3600).
pub(crate) const MAX_FILE_PIXELS: u64 = 1 << 28;
/// What [`decode`] says of a bitmap past what is left of the file's budget.
pub(crate) const OVER_BUDGET: &str = "the bitmaps hold more pixels than one file may";

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const BI_JPEG: u32 = 4;
const BI_PNG: u32 = 5;

/// Pixels with straight (not premultiplied) alpha, row by row from the top.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rgba {
    /// `img`, scaled down to what a metafile stores when it is larger.
    pub fn fitted(img: image::RgbaImage) -> Self {
        let (w, h) = img.dimensions();
        let side = f64::from(w.max(h)) / f64::from(MAX_SIDE);
        let area = (u64::from(w) * u64::from(h)) as f64 / MAX_PIXELS as f64;
        let k = side.max(area.sqrt());
        let img = if k > 1.0 {
            let nw = ((f64::from(w) / k).floor() as u32).max(1);
            let nh = ((f64::from(h) / k).floor() as u32).max(1);
            image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
        } else {
            img
        };
        let (width, height) = img.dimensions();
        Self { width, height, pixels: img.into_raw() }
    }

    /// Every pixel fully opaque?
    pub fn opaque(&self) -> bool {
        self.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255)
    }

    /// As PNG bytes.
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        let img = image::RgbaImage::from_raw(self.width, self.height, self.pixels.clone()).ok_or("the image's pixels don't match its size")?;
        let mut png = vec![];
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        Ok(png)
    }
}

/// A BITMAPINFOHEADER of a bottom-up, uncompressed `width` × `height` bitmap of `bits` per pixel.
fn header(width: u32, height: u32, bits: u16, size: usize) -> Vec<u8> {
    let mut h = crate::bytes::Out::default();
    h.u32(40);
    h.i32(i32::try_from(width).unwrap_or(i32::MAX));
    h.i32(i32::try_from(height).unwrap_or(i32::MAX));
    h.u16(1);
    h.u16(bits);
    h.u32(BI_RGB);
    h.u32(u32::try_from(size).unwrap_or(0));
    // 2835 pixels a metre: 72 ppi (the size on the page comes from the record).
    h.i32(2835);
    h.i32(2835);
    h.u32(0);
    h.u32(0);
    h.0
}

/// The bytes of a row of `width` pixels of `bits` each, padded to 4 bytes.
fn stride(width: u32, bits: u32) -> Option<usize> {
    let row_bits = u64::from(width).checked_mul(u64::from(bits))?;
    usize::try_from(row_bits.div_ceil(32) * 4).ok()
}

/// `img` at `opacity`, composited over white, as a 24-bit DIB → (header, bits).
pub(crate) fn rgb24(img: &Rgba, opacity: f32) -> (Vec<u8>, Vec<u8>) {
    let row = stride(img.width, 24).unwrap_or(0);
    let mut bits = vec![0u8; row * img.height as usize];
    let over = |c: u8, a: u32| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
    for (y, src) in img.pixels.chunks_exact((img.width as usize * 4).max(1)).enumerate() {
        // Bottom-up: the first stored row is the image's last.
        let Some(at) = (img.height as usize).checked_sub(y + 1).map(|r| r * row) else { continue };
        let Some(dst) = bits.get_mut(at..at + img.width as usize * 3) else { continue };
        for (d, p) in dst.as_chunks_mut::<3>().0.iter_mut().zip(src.as_chunks::<4>().0) {
            let a = (f32::from(p[3]) * opacity.clamp(0.0, 1.0)).round() as u32;
            d.copy_from_slice(&[over(p[2], a), over(p[1], a), over(p[0], a)]);
        }
    }
    (header(img.width, img.height, 24, bits.len()), bits)
}

/// `img` at `opacity` as a 32-bit DIB of premultiplied BGRA → (header, bits).
pub(crate) fn bgra32(img: &Rgba, opacity: f32) -> (Vec<u8>, Vec<u8>) {
    let row = img.width as usize * 4;
    let mut bits = vec![0u8; row * img.height as usize];
    for (y, src) in img.pixels.chunks_exact(row.max(1)).enumerate() {
        let Some(at) = (img.height as usize).checked_sub(y + 1).map(|r| r * row) else { continue };
        let Some(dst) = bits.get_mut(at..at + row) else { continue };
        for (d, p) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
            let a = (f32::from(p[3]) * opacity.clamp(0.0, 1.0)).round() as u32;
            let pre = |c: u8| ((u32::from(c) * a + 127) / 255) as u8;
            d.copy_from_slice(&[pre(p[2]), pre(p[1]), pre(p[0]), a as u8]);
        }
    }
    (header(img.width, img.height, 32, bits.len()), bits)
}

/// How the fourth byte of 32-bit pixels reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Alpha {
    /// Unused (StretchDIBits and BitBlt ignore it): every pixel opaque.
    Ignore,
    /// Premultiplied alpha (AlphaBlend with per-pixel alpha).
    Premultiplied,
}

/// A colour channel of `v` under `mask`, scaled to 0–255.
fn channel(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = u64::from(mask >> shift);
    let x = u64::from((v & mask) >> shift);
    ((x * 255 + max / 2) / max.max(1)).min(255) as u8
}

/// Take a bitmap of `w` × `h` from what is `left` of the file's budget, before it is decoded.
fn charge(w: u32, h: u32, left: &mut u64) -> Result<(), String> {
    let n = u64::from(w) * u64::from(h);
    *left = left.checked_sub(n).ok_or(OVER_BUDGET)?;
    Ok(())
}

/// Read a DIB from its BITMAPINFO (`bmi`: the header and colour table) and pixel `bits`, taking its
/// pixels from what is `left` of the file's budget ([`MAX_FILE_PIXELS`]).
pub(crate) fn decode(bmi: &[u8], bits: &[u8], alpha: Alpha, left: &mut u64) -> Result<Rgba, String> {
    let bad = || "a bitmap is damaged".to_string();
    let mut r = Reader::new(bmi);
    let size = r.u32().ok_or_else(bad)?;
    let (width, height, bpp, compression, colors, palette_entry, masks_at) = if size == 12 {
        // BITMAPCOREHEADER: 16-bit sizes, RGB triples.
        let w = i32::from(r.u16().ok_or_else(bad)?);
        let h = i32::from(r.u16().ok_or_else(bad)?);
        r.skip(2).ok_or_else(bad)?;
        let bpp = r.u16().ok_or_else(bad)?;
        (w, h, bpp, BI_RGB, 0, 3, 0)
    } else if size >= 40 {
        let w = r.i32().ok_or_else(bad)?;
        let h = r.i32().ok_or_else(bad)?;
        r.skip(2).ok_or_else(bad)?;
        let bpp = r.u16().ok_or_else(bad)?;
        let c = r.u32().ok_or_else(bad)?;
        r.skip(12).ok_or_else(bad)?;
        let used = r.u32().ok_or_else(bad)?;
        (w, h, bpp, c, used, 4, 40)
    } else {
        return Err(bad());
    };
    if compression == BI_JPEG || compression == BI_PNG {
        // The size comes from the image header, so an over-limit bitmap is refused before any pixels are decoded.
        let (w, h) = image::ImageReader::new(Cursor::new(bits)).with_guessed_format().map_err(|_| bad())?.into_dimensions().map_err(|_| bad())?;
        if w > MAX_READ_SIDE || h > MAX_READ_SIDE || u64::from(w) * u64::from(h) > MAX_READ_PIXELS {
            return Err("a bitmap is too large to read".into());
        }
        charge(w, h, left)?;
        let img = image::load_from_memory(bits).map_err(|_| bad())?.to_rgba8();
        return Ok(Rgba { width: w, height: h, pixels: img.into_raw() });
    }
    if compression != BI_RGB && compression != BI_BITFIELDS {
        return Err("compressed (RLE) bitmaps can't be read yet".into());
    }
    let top_down = height < 0;
    let (w, h) = (width.unsigned_abs(), height.unsigned_abs());
    if w == 0 || h == 0 {
        return Err("a bitmap is empty".into());
    }
    if w > MAX_READ_SIDE || h > MAX_READ_SIDE || u64::from(w) * u64::from(h) > MAX_READ_PIXELS {
        return Err("a bitmap is too large to read".into());
    }
    let header_len = size as usize;
    // Bit-field masks follow a plain 40-byte header, or sit inside the larger ones.
    let fields = compression == BI_BITFIELDS;
    let masks: Option<[u32; 4]> = fields.then(|| {
        let mut m = Reader::new(bmi.get(masks_at..).unwrap_or_default());
        let rgb = [m.u32().unwrap_or(0), m.u32().unwrap_or(0), m.u32().unwrap_or(0)];
        // An alpha mask comes with BITMAPV3INFOHEADER and later.
        let a = if header_len >= 56 { m.u32().unwrap_or(0) } else { 0 };
        [rgb[0], rgb[1], rgb[2], a]
    });
    let palette_at = header_len + if fields && header_len == 40 { 12 } else { 0 };
    let bpp_u = u32::from(bpp);
    let palette: Vec<[u8; 3]> = if bpp_u <= 8 {
        let n = if colors == 0 { 1usize << bpp_u } else { (colors as usize).min(256) };
        bmi.get(palette_at..)
            .unwrap_or_default()
            .chunks_exact(palette_entry)
            .take(n)
            .map(|e| [e.get(2).copied().unwrap_or(0), e.get(1).copied().unwrap_or(0), e.first().copied().unwrap_or(0)])
            .collect()
    } else {
        vec![]
    };
    if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
        return Err(format!("{bpp}-bit bitmaps can't be read"));
    }
    let row = stride(w, bpp_u).ok_or_else(bad)?;
    if bits.len() < row.checked_mul(h as usize).ok_or_else(bad)? {
        return Err(bad());
    }
    charge(w, h, left)?;
    let (mr, mg, mb, ma) = match (masks, bpp) {
        (Some([r, g, b, a]), _) => (r, g, b, a),
        (None, 16) => (0x7c00, 0x03e0, 0x001f, 0),
        _ => (0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000),
    };
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        let src_row = if top_down { y } else { h as usize - 1 - y };
        let line = bits.get(src_row * row..src_row * row + row).ok_or_else(bad)?;
        for x in 0..w as usize {
            let px: [u8; 4] = match bpp {
                1 | 4 | 8 => {
                    let bit = x * bpp_u as usize;
                    let byte = line.get(bit / 8).copied().unwrap_or(0);
                    let idx = (byte >> (8 - bpp_u as usize - bit % 8)) & ((1u16 << bpp_u) - 1) as u8;
                    let [r, g, b] = palette.get(idx as usize).copied().unwrap_or([0, 0, 0]);
                    [r, g, b, 255]
                }
                16 => {
                    let v = u32::from(u16::from_le_bytes([line.get(x * 2).copied().unwrap_or(0), line.get(x * 2 + 1).copied().unwrap_or(0)]));
                    [channel(v, mr), channel(v, mg), channel(v, mb), 255]
                }
                24 => {
                    let p = line.get(x * 3..x * 3 + 3).unwrap_or(&[0, 0, 0]);
                    [p[2], p[1], p[0], 255]
                }
                _ => {
                    let p = line.get(x * 4..x * 4 + 4).unwrap_or(&[0, 0, 0, 0]);
                    let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                    let a = if alpha == Alpha::Premultiplied && ma != 0 { channel(v, ma) } else { 255 };
                    let un =
                        |c: u8| if alpha == Alpha::Premultiplied && a < 255 { (u32::from(c) * 255 / u32::from(a.max(1))).min(255) as u8 } else { c };
                    [un(channel(v, mr)), un(channel(v, mg)), un(channel(v, mb)), if alpha == Alpha::Premultiplied { a } else { 255 }]
                }
            };
            pixels.extend_from_slice(&px);
        }
    }
    Ok(Rgba { width: w, height: h, pixels })
}

/// A packed DIB (a BITMAPINFO and its bits, one after the other, as WMF records hold them) split
/// into the two.
pub(crate) fn split(packed: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut r = Reader::new(packed);
    let size = r.u32()? as usize;
    let (bpp, compression, used, entry) = if size == 12 {
        r.skip(6)?;
        (r.u16()?, BI_RGB, 0, 3)
    } else {
        r.skip(10)?;
        let bpp = r.u16()?;
        let c = r.u32()?;
        r.skip(12)?;
        (bpp, c, r.u32()? as usize, 4)
    };
    let masks = if compression == BI_BITFIELDS && size == 40 { 12 } else { 0 };
    let colors = if used > 0 {
        used.min(256)
    } else if bpp <= 8 {
        1usize << bpp
    } else {
        0
    };
    let len = size.checked_add(masks)?.checked_add(colors * entry)?;
    Some((packed.get(..len)?, packed.get(len..)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Rgba {
        // 3 × 2: red, green, blue / half-transparent black, transparent, white.
        let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255];
        Rgba { width: 3, height: 2, pixels }
    }

    /// A whole file's budget.
    fn budget() -> u64 {
        MAX_FILE_PIXELS
    }

    /// A BITMAPINFOHEADER of an embedded PNG.
    fn png_header() -> Vec<u8> {
        let mut bmi = vec![0u8; 40];
        bmi[..4].copy_from_slice(&40u32.to_le_bytes());
        bmi[16..20].copy_from_slice(&BI_PNG.to_le_bytes());
        bmi
    }

    #[test]
    fn oversized_png_is_refused_from_its_header() {
        // A width over the limit in the IHDR chunk: decoding the pixels would fail with a different error.
        let png = crate::tests::png_claiming(MAX_READ_SIDE + 1, 1);
        let err = decode(&png_header(), &png, Alpha::Ignore, &mut budget()).unwrap_err();
        assert_eq!(err, "a bitmap is too large to read");
    }

    #[test]
    fn bitmaps_share_the_file_budget() {
        // #3600: each bitmap is charged before it is decoded, and one past what is left is refused.
        let (bmi, bits) = rgb24(&sample(), 1.0);
        let mut left = 6 + 5;
        assert!(decode(&bmi, &bits, Alpha::Ignore, &mut left).is_ok());
        assert_eq!(left, 5);
        assert_eq!(decode(&bmi, &bits, Alpha::Ignore, &mut left).unwrap_err(), OVER_BUDGET);
        assert_eq!(left, 5, "a refused bitmap costs nothing");
        // An embedded PNG is charged from its header, so one past the budget is never decoded.
        let mut left = (1 << 20) - 1;
        let png = crate::tests::png_claiming(1 << 10, 1 << 10);
        assert_eq!(decode(&png_header(), &png, Alpha::Ignore, &mut left).unwrap_err(), OVER_BUDGET);
    }

    #[test]
    fn rgb24_is_bottom_up_bgr_over_white() {
        let (bmi, bits) = rgb24(&sample(), 1.0);
        assert_eq!(bmi.len(), 40);
        assert_eq!(&bmi[14..16], &[24, 0], "24 bits a pixel");
        // Rows of 9 bytes padded to 12, the last row first: half-transparent black over white is
        // mid grey, transparent is white.
        assert_eq!(bits, vec![127, 127, 127, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn bgra32_is_premultiplied() {
        let (bmi, bits) = bgra32(&sample(), 1.0);
        assert_eq!(&bmi[14..16], &[32, 0]);
        assert_eq!(&bits[..12], &[0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255]);
        assert_eq!(&bits[12..16], &[0, 0, 255, 255], "red, as BGRA");
        // At half opacity.
        let (_, half) = bgra32(&sample(), 0.5);
        assert_eq!(&half[12..16], &[0, 0, 128, 128]);
    }

    #[test]
    fn rgb24_round_trips_over_white() {
        let (bmi, bits) = rgb24(&sample(), 1.0);
        assert_eq!(bmi.len(), 40);
        // Rows of 9 bytes padded to 12.
        assert_eq!(bits.len(), 24);
        let back = decode(&bmi, &bits, Alpha::Ignore, &mut budget()).unwrap();
        assert_eq!((back.width, back.height), (3, 2));
        assert_eq!(&back.pixels[..12], &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]);
        // Half-transparent black over white is mid grey; transparent is white.
        assert_eq!(&back.pixels[12..16], &[127, 127, 127, 255]);
        assert_eq!(&back.pixels[16..20], &[255, 255, 255, 255]);
    }

    #[test]
    fn bgra32_round_trips_with_alpha() {
        let (bmi, bits) = bgra32(&sample(), 1.0);
        let back = decode(&bmi, &bits, Alpha::Premultiplied, &mut budget()).unwrap();
        assert_eq!(&back.pixels[..12], &sample().pixels[..12]);
        assert_eq!(back.pixels[15], 128);
        assert_eq!(back.pixels[19], 0);
        // Read as StretchDIBits does, alpha is ignored.
        assert!(decode(&bmi, &bits, Alpha::Ignore, &mut budget()).unwrap().opaque());
    }

    #[test]
    fn palette_bitmaps_and_damage() {
        // A 1-bit 2 × 1 bitmap: black then white.
        let mut bmi = header(2, 1, 1, 4);
        bmi.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]);
        let back = decode(&bmi, &[0b0100_0000, 0, 0, 0], Alpha::Ignore, &mut budget()).unwrap();
        assert_eq!(back.pixels, vec![0, 0, 0, 255, 255, 255, 255, 255]);
        // Too few bits, a silly size, an unknown depth: errors, no panic.
        assert!(decode(&bmi, &[0], Alpha::Ignore, &mut budget()).is_err());
        assert!(decode(&header(1 << 20, 1 << 20, 24, 0), &[], Alpha::Ignore, &mut budget()).is_err());
        assert!(decode(&header(1, 1, 7, 0), &[0; 4], Alpha::Ignore, &mut budget()).is_err());
        assert!(decode(&[1, 2], &[], Alpha::Ignore, &mut budget()).is_err());
    }

    #[test]
    fn large_images_are_scaled_to_fit() {
        let img = image::RgbaImage::new(MAX_SIDE * 2, 4);
        let f = Rgba::fitted(img);
        assert_eq!((f.width, f.height), (MAX_SIDE, 2));
    }
}
