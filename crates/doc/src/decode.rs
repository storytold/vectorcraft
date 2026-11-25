//! Images decoded through craft-image-codecs as the image crate gave them (#778): the renderer,
//! File › Open and Place, SVG import, and embedded and linked images read their pixels with
//! [`decode_image`] and [`decode_within`], and size them from their header with [`decoded_layout`].

/// Decoded pixels, as stored and as the image crate gave them, and the ICC profile the file embeds.
pub type DecodedImage = (image::DynamicImage, Option<Vec<u8>>);

/// `bytes` decoded as `format` within `allowance` bytes, the pixels as stored (not turned
/// upright) and as the image crate gave them ([`dynamic`]), with the ICC profile the file embeds.
/// The renderer, File › Open and Place, SVG import, and embedded and linked images read images
/// with it.
pub fn decode_image(format: craft_image_codecs::Format, bytes: &[u8], allowance: u64) -> Result<DecodedImage, craft_image_codecs::CodecError> {
    let opts = craft_image_codecs::DecodeOptions {
        limits: craft_image_codecs::Limits { max_width: u32::MAX, max_height: u32::MAX, max_pixels: u64::MAX, max_alloc: allowance },
        keep_orientation: true,
    };
    let mut img = craft_image_codecs::decode_as_with(format, bytes, &opts)?;
    let icc = img.icc.take();
    let img = dynamic(img).ok_or_else(|| craft_image_codecs::CodecError::InvalidImage("the decoded samples don't fit the image".into()))?;
    Ok((img, icc))
}

/// `bytes` decoded within `max` bytes, counted as the file's samples and as RGBA (#1717), the
/// pixels as stored and as the image crate gave them, with the ICC profile the file embeds.
/// Image blobs read their pixels with it. `Ok(None)` when they would take more than `max` as
/// samples or as RGBA (the header alone decides; the pixels are not read). `Err` when the bytes
/// aren't an image the decoder reads, including a file whose header fits the allowance but whose
/// pixels can't be decoded (a cut-short file, or one the decoder refuses as a decompression bomb).
pub fn decode_within(bytes: &[u8], max: u64) -> Result<Option<DecodedImage>, craft_image_codecs::CodecError> {
    let format = craft_image_codecs::detect(bytes).ok_or(craft_image_codecs::CodecError::UnknownFormat)?;
    let (w, h, per_pixel) =
        decoded_layout(bytes, format).ok_or_else(|| craft_image_codecs::CodecError::InvalidImage("the image's size can't be read".into()))?;
    let pixels = u64::from(w) * u64::from(h);
    if pixels.saturating_mul(per_pixel) > max || pixels.saturating_mul(4) > max {
        return Ok(None);
    }
    decode_image(format, bytes, max).map(Some)
}

/// A decoded image as the image crate's: 8- and 16-bit gray, gray and alpha, RGB and RGBA as they
/// are, 32-bit float RGB and RGBA as they are, anything else as 8-bit RGBA.
fn dynamic(img: craft_image_codecs::Image) -> Option<image::DynamicImage> {
    use craft_image_codecs::{ChannelLayout as L, SampleType as S};
    use image::{DynamicImage as D, ImageBuffer as B};
    let (w, h) = img.dimensions();
    Some(match (img.layout(), img.sample_type()) {
        (L::Gray, S::U8) => D::ImageLuma8(B::from_raw(w, h, img.into_data())?),
        (L::GrayA, S::U8) => D::ImageLumaA8(B::from_raw(w, h, img.into_data())?),
        (L::Rgb, S::U8) => D::ImageRgb8(B::from_raw(w, h, img.into_data())?),
        (L::Rgba, S::U8) => D::ImageRgba8(B::from_raw(w, h, img.into_data())?),
        (layout @ (L::Gray | L::GrayA | L::Rgb | L::Rgba), S::U16) => {
            let v = img.to_u16_samples()?;
            match layout {
                L::Gray => D::ImageLuma16(B::from_raw(w, h, v)?),
                L::GrayA => D::ImageLumaA16(B::from_raw(w, h, v)?),
                L::Rgb => D::ImageRgb16(B::from_raw(w, h, v)?),
                _ => D::ImageRgba16(B::from_raw(w, h, v)?),
            }
        }
        _ => D::ImageRgba8(B::from_raw(w, h, img.to_rgba8())?),
    })
}

/// The image's size and the bytes a pixel of it takes decoded ([`decode_as`]), read from its
/// header: palettes as RGB (RGBA with a transparent entry; GIF frames always RGBA), bitmaps as 8-bit
/// gray, 16-bit samples as two bytes. A PNG whose header alone is there is read from its `IHDR`;
/// a format whose header says only its size (TGA, ICO and the like) counts RGBA.
pub fn decoded_layout(bytes: &[u8], format: craft_image_codecs::Format) -> Option<(u32, u32, u64)> {
    use craft_image_codecs::{Format as F, RasterColorType as T};
    if let Some(i) = craft_image_codecs::inspect_raster(bytes).ok().flatten() {
        let deep = i.bits_per_sample > 8 && i.color_type != T::Indexed;
        let channels = match i.color_type {
            _ if format == F::Gif => 4,
            T::Indexed => 3 + u64::from(i.transparent_index.is_some()),
            T::Bitmap => 1,
            _ => u64::from(i.channels.max(1)),
        };
        return Some((i.width, i.height, channels * if deep { 2 } else { 1 }));
    }
    if format != F::Png {
        return crate::header_size(bytes).map(|(w, h)| (w, h, 4));
    }
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let be = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let (depth, color) = (*bytes.get(24)?, *bytes.get(25)?);
    let channels: u64 = match color {
        0 => 1,
        2 | 3 => 3,
        4 => 2,
        6 => 4,
        _ => return None,
    };
    Some((be(16)?, be(20)?, channels * if depth == 16 { 2 } else { 1 }))
}
