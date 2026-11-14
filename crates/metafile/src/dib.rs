//! Device-independent bitmaps (DIBs), the pixels inside image records: written bottom-up as 24-bit
//! (opaque, composited over white) or 32-bit premultiplied BGRA (AlphaBlend's source).

use std::io::Cursor;

/// Most pixels along a side of an image the writers store (larger ones are scaled down).
const MAX_SIDE: u32 = 8192;
/// Most pixels an image the writers store holds.
const MAX_PIXELS: u64 = 1 << 24;
/// Most pixels along a side, and in all, of a bitmap the PICT reader decodes.
pub(crate) const MAX_READ_SIDE: u32 = 1 << 15;
pub(crate) const MAX_READ_PIXELS: u64 = 1 << 26;
/// Most pixels the bitmaps of one file may decode to together (four of the largest one), so a small
/// file of many image records can't cost minutes of decoding (#3600).
pub(crate) const MAX_FILE_PIXELS: u64 = 1 << 28;
/// What [`charge`] says of a bitmap past what is left of the file's budget.
pub(crate) const OVER_BUDGET: &str = "the bitmaps hold more pixels than one file may";

const BI_RGB: u32 = 0;

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

/// Take a bitmap of `w` × `h` from what is `left` of the file's budget, before it is decoded.
pub(crate) fn charge(w: u32, h: u32, left: &mut u64) -> Result<(), String> {
    let n = u64::from(w) * u64::from(h);
    *left = left.checked_sub(n).ok_or(OVER_BUDGET)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Rgba {
        // 3 × 2: red, green, blue / half-transparent black, transparent, white.
        let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255];
        Rgba { width: 3, height: 2, pixels }
    }

    #[test]
    fn bitmaps_share_the_file_budget() {
        // #3600: each bitmap is charged before it is decoded, and one past what is left is refused.
        let mut left = 6 + 5;
        assert!(charge(3, 2, &mut left).is_ok());
        assert_eq!(left, 5);
        assert_eq!(charge(3, 2, &mut left).unwrap_err(), OVER_BUDGET);
        assert_eq!(left, 5, "a refused bitmap costs nothing");
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
    fn large_images_are_scaled_to_fit() {
        let img = image::RgbaImage::new(MAX_SIDE * 2, 4);
        let f = Rgba::fitted(img);
        assert_eq!((f.width, f.height), (MAX_SIDE, 2));
    }
}
