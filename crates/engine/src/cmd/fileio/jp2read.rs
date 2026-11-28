//! JPEG 2000 JP2 and codestream images for File › Open and File › Place. The decoded image is
//! bounded before allocation and converted to RGBA for the common PNG storage path.

use craft_image_codecs::{ChannelLayout, CodecError, Format, Limits};
use vectorcraft_color::cms::{IccProfile, Intent};

/// Recognize a JP2 signature box or the start of a raw codestream. The decoder checks the rest.
pub(super) fn is_jpeg2000(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\0\0\0\x0cjP  \r\n\x87\n") || bytes.starts_with(b"\xff\x4f")
}

/// Boxes at one JP2 nesting level. Invalid lengths stop the walk at that level.
pub(super) fn boxes(b: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let header = b.get(at..at.checked_add(8)?)?;
        let size = u32::from_be_bytes(header.get(..4)?.try_into().ok()?);
        let (length, head) = match size {
            0 => (b.len().checked_sub(at)?, 8usize),
            1 => (usize::try_from(u64::from_be_bytes(b.get(at.checked_add(8)?..at.checked_add(16)?)?.try_into().ok()?)).ok()?, 16usize),
            n => (n as usize, 8usize),
        };
        if length < head {
            return None;
        }
        let end = at.checked_add(length)?;
        let data = b.get(at.checked_add(head)?..end)?;
        let ty = header.get(4..8)?;
        at = end;
        Some((ty, data))
    })
}

/// Decode `bytes` to RGBA, checking memory limits and converting embedded ICC/CMYK color spaces
/// to sRGB via `vectorcraft_color`.
pub(super) fn decode(bytes: &[u8], max_alloc: u64) -> Result<image::RgbaImage, String> {
    if !is_jpeg2000(bytes) {
        return Err("the JPEG 2000 file can't be read".into());
    }
    let limits = Limits { max_alloc, max_pixels: max_alloc / 4, ..Limits::default() };
    let opts = craft_image_codecs::DecodeOptions { limits, ..craft_image_codecs::DecodeOptions::default() };
    let image = craft_image_codecs::decode_as_with(Format::Jp2, bytes, &opts).map_err(|e| match e {
        CodecError::LimitExceeded(_) => "an image is too large to draw".to_string(),
        CodecError::Unsupported { .. } => "the JPEG 2000 file has an unsupported color space or JPX content".to_string(),
        _ => "the JPEG 2000 file can't be read".to_string(),
    })?;
    let (w, h) = (image.width(), image.height());
    let too_large = || format!("an image of {w} × {h} pixels is too large to draw");
    let pixels = u64::from(w).checked_mul(u64::from(h)).ok_or_else(too_large)?;
    let rgba_size = pixels.checked_mul(4).ok_or_else(too_large)?;
    if rgba_size > max_alloc {
        return Err(too_large());
    }
    let layout = image.layout();
    let (channels, has_alpha) = match layout {
        ChannelLayout::Gray => (1usize, false),
        ChannelLayout::GrayA => (1, true),
        ChannelLayout::Rgb => (3, false),
        ChannelLayout::Rgba => (3, true),
        ChannelLayout::Cmyk => (4, false),
        ChannelLayout::CmykA => (4, true),
        _ => return Err("the JPEG 2000 file has an unsupported color space or JPX content".into()),
    };
    let stride = channels + usize::from(has_alpha);
    let profile = match image.icc.as_deref() {
        Some(icc) => Some(IccProfile::from_bytes(None, icc).map_err(|_| "the JPEG 2000 file has an unsupported color space or JPX content")?),
        None => None,
    };
    let cms = vectorcraft_color::cms::active();
    let mut rgba = Vec::with_capacity(usize::try_from(rgba_size).map_err(|_| too_large())?);
    for sample in image.data().chunks_exact(stride) {
        let at = |i: usize| sample.get(i).copied().unwrap_or(0);
        let rgb = if let Some(profile) = profile.as_ref() {
            let mut values = [0.0f32; 4];
            for (dst, src) in values.iter_mut().zip(sample.iter().take(channels)) {
                *dst = f32::from(*src) / 255.0;
            }
            profile
                .to_srgb_into(&values[..channels], Intent::RelativeColorimetric)
                .ok_or("the JPEG 2000 file has an unsupported color space or JPX content")?
                .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        } else {
            match layout {
                ChannelLayout::Gray | ChannelLayout::GrayA => [at(0); 3],
                ChannelLayout::Rgb | ChannelLayout::Rgba => [at(0), at(1), at(2)],
                ChannelLayout::Cmyk | ChannelLayout::CmykA => cms
                    .cmyk_to_srgb([at(0), at(1), at(2), at(3)].map(|v| f32::from(v) / 255.0), false)
                    .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8),
                _ => return Err("the JPEG 2000 file has an unsupported color space or JPX content".into()),
            }
        };
        let [r, g, b] = rgb;
        rgba.extend_from_slice(&[r, g, b, if has_alpha { at(channels) } else { 255 }]);
    }
    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| "the JPEG 2000 file can't be read".into())
}
