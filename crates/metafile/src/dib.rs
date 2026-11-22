//! App bitmap sizing, read budgets, and raster conversion for imported pictures.

use craft_metafile::write::Image;
use vectorcraft_doc::resample::{Resample, resized};

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

/// Pixels with straight (not premultiplied) alpha, row by row from the top.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// `img`, scaled down to what a metafile stores when it is larger.
pub(crate) fn fitted(img: image::RgbaImage) -> Image {
    let (w, h) = img.dimensions();
    let side = f64::from(w.max(h)) / f64::from(MAX_SIDE);
    let area = (u64::from(w) * u64::from(h)) as f64 / MAX_PIXELS as f64;
    let k = side.max(area.sqrt());
    let img = if k > 1.0 {
        let nw = ((f64::from(w) / k).floor() as u32).max(1);
        let nh = ((f64::from(h) / k).floor() as u32).max(1);
        let how = Resample::Filter(image::imageops::FilterType::Triangle);
        resized(image::DynamicImage::ImageRgba8(img), nw, nh, how).into_rgba8()
    } else {
        img
    };
    let (width, height) = img.dimensions();
    Image { width, height, rgba: img.into_raw() }
}

impl Rgba {
    /// As PNG bytes.
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        let img = craft_image_codecs::Image::from_u8(self.width, self.height, craft_image_codecs::ChannelLayout::Rgba, self.pixels.clone())
            .map_err(|_| "the image's pixels don't match its size")?;
        craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).map_err(|e| e.to_string())
    }
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

    fn sample() -> Image {
        // 3 × 2: red, green, blue / half-transparent black, transparent, white.
        let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255];
        Image { width: 3, height: 2, rgba: pixels }
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
        // WMF's shared writer emits a StretchDIB record: 28 bytes of parameters and
        // record framing precede BITMAPINFOHEADER, then the bottom-up rows.
        let image = std::sync::Arc::new(sample());
        let op = craft_metafile::write::Op::Image { image, xf: kurbo::Affine::IDENTITY, opacity: 1.0 };
        let bytes = craft_metafile::write::wmf(&[op], kurbo::Rect::new(0.0, 0.0, 3.0, 2.0)).unwrap().bytes;
        let records = crate::tests::wmf_records(&bytes);
        let (_, record) = records.iter().find(|(kind, _)| *kind == 0x0f43).unwrap();
        let (bmi, bits) = (&record[28..68], &record[68..]);
        assert_eq!(bmi.len(), 40);
        assert_eq!(&bmi[14..16], &[24, 0], "24 bits a pixel");
        // Rows of 9 bytes padded to 12, the last row first: half-transparent black over white is
        // mid grey, transparent is white.
        assert_eq!(bits, vec![127, 127, 127, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn bgra32_is_premultiplied() {
        let pixels = |opacity| {
            let image = std::sync::Arc::new(sample());
            let op = craft_metafile::write::Op::Image { image, xf: kurbo::Affine::IDENTITY, opacity };
            let bytes = craft_metafile::write::emf(&[op], kurbo::Rect::new(0.0, 0.0, 3.0, 2.0), "VectorCraft", "").unwrap();
            let records = crate::tests::emf_records(&bytes);
            let (_, record) = records.iter().find(|(kind, _)| *kind == 114).unwrap();
            let (bmi, bits) = (&record[108..148], &record[148..]);
            assert_eq!(&bmi[14..16], &[32, 0]);
            bits.to_vec()
        };
        let bits = pixels(1.0);
        assert_eq!(&bits[..12], &[0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255]);
        assert_eq!(&bits[12..16], &[0, 0, 255, 255], "red, as BGRA");
        // At half opacity.
        let half = pixels(0.5);
        assert_eq!(&half[12..16], &[0, 0, 128, 128]);
    }

    #[test]
    fn large_images_are_scaled_to_fit() {
        let img = image::RgbaImage::new(MAX_SIDE * 2, 4);
        let f = fitted(img);
        assert_eq!((f.width, f.height), (MAX_SIDE, 2));
    }

    /// A large image scaled to fit keeps the color of its opaque pixels beside clear ones: the
    /// clear black beside them does not darken them.
    #[test]
    fn large_images_keep_their_color_beside_clear_pixels() {
        let img =
            image::RgbaImage::from_fn(MAX_SIDE * 2, 4, |x, _| if x % 2 == 0 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([0, 0, 0, 0]) });
        let f = fitted(img);
        assert_eq!((f.width, f.height), (MAX_SIDE, 2));
        let m = |c: u8, a: u8| (u16::from(c) * u16::from(a) / 255) as u8;
        for p in f.rgba.as_chunks::<4>().0 {
            assert!(p[3] > 0 && [200, 30, 30].iter().zip(p).all(|(&want, &c)| m(c, p[3]).abs_diff(m(want, p[3])) <= 1), "{p:?}");
        }
    }
}
