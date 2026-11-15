//! Reading JPEG XL images (`.jxl`, ISO/IEC 18181) for File › Open and File › Place, with
//! `craft-jxl`: the bare codestream and the ISOBMFF container. The first frame of an animation is
//! read, with the orientation its header gives applied.

use craft_jxl::{DecodeLimits, Error as JxlError, Layout};

/// A bare JPEG XL codestream starts with these bytes.
const CODESTREAM: [u8; 2] = [0xff, 0x0a];
/// The signature box an ISOBMFF JPEG XL container starts with.
const CONTAINER: [u8; 12] = [0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a];
/// Buffer allowance for container box filtering.
const HEADER_BUFFERS: usize = 16 << 20;

/// Is `bytes` a JPEG XL image (a bare codestream or a container)?
pub fn is_jxl(bytes: &[u8]) -> bool {
    bytes.starts_with(&CODESTREAM) || bytes.starts_with(&CONTAINER)
}

/// Resolution after the decoder applies the image header's orientation.
pub(crate) fn oriented_resolution(ppi: Option<(f64, f64)>, orientation: u32) -> Option<(f64, f64)> {
    ppi.map(|(x, y)| if (5..=8).contains(&orientation) { (y, x) } else { (x, y) })
}

/// The first frame of a JPEG XL image as straight RGBA, with the embedded ICC profile its colors
/// are in when they are not converted to sRGB, and the header orientation.
pub fn decode(bytes: &[u8], max_alloc: u64) -> Result<(image::RgbaImage, Option<Vec<u8>>, u32), String> {
    let pixel_boxes = pixel_boxes(bytes, max_alloc.max(HEADER_BUFFERS as u64))?;
    let bytes = pixel_boxes.as_deref().unwrap_or(bytes);
    let limits = DecodeLimits { max_pixels: max_alloc / 4, max_tracked_bytes: Some(max_alloc), max_input_bytes: usize::MAX };
    let decoded = craft_jxl::decode(bytes, limits).map_err(|e| match e {
        JxlError::PixelLimitExceeded { width, height, .. } => too_large(width, height),
        JxlError::LimitExceeded(msg) => format!("an image is too large to draw: {msg}"),
        JxlError::Unsupported(msg) if msg.contains("CMYK") => "CMYK JPEG XL images can't be read".into(),
        other => unreadable(other),
    })?;
    let (w, h) = (decoded.width, decoded.height);
    if u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4) > max_alloc {
        return Err(too_large(w, h));
    }
    let profile = if !decoded.xyb_encoded && decoded.metadata.icc.as_deref() == Some(decoded.rendered_icc.as_slice()) {
        if matches!(decoded.layout, Layout::Gray | Layout::GrayAlpha) {
            return Err(unreadable("unsupported grayscale ICC profile"));
        }
        if decoded.rendered_icc.get(16..20) != Some(b"RGB ") {
            return Err(unreadable("unsupported ICC profile color space"));
        }
        Some(decoded.rendered_icc)
    } else {
        None
    };
    let channels = decoded.layout.channels();
    let gray = channels < 3;
    let has_alpha = decoded.layout.has_alpha();
    let mut out = image::RgbaImage::new(w, h);
    for (dst, src) in out.as_chunks_mut::<4>().0.iter_mut().zip(decoded.pixels.chunks_exact(channels)) {
        let a = if has_alpha { src[channels - 1].clamp(0.0, 1.0) } else { 1.0 };
        let unpremul = |v: f32| if decoded.pixels_premultiplied && a > 0.0 { v / a } else { v };
        let rgb = if gray {
            let v = byte(unpremul(src[0]));
            [v, v, v]
        } else {
            [byte(unpremul(src[0])), byte(unpremul(src[1])), byte(unpremul(src[2]))]
        };
        dst.copy_from_slice(&[rgb[0], rgb[1], rgb[2], byte(a)]);
    }
    Ok((out, profile, decoded.source_orientation))
}

/// A container's signature and the boxes the decoder reads pixels from (`ftyp`, `jxll`, `jxlc` and
/// `jxlp`). Every other box is left out: Exif, XMP, JPEG reconstruction data and Brotli-compressed
/// `brob` boxes, which the decoder would otherwise store and unpack without a size limit. A box
/// cut short ends the file. Selected boxes exceeding `max_copy` are refused with "the JPEG XL
/// container is too large to read". `None` for a bare codestream.
pub(super) fn pixel_boxes(bytes: &[u8], max_copy: u64) -> Result<Option<Vec<u8>>, String> {
    let Some(mut rest) = bytes.strip_prefix(&CONTAINER) else { return Ok(None) };
    let mut out = CONTAINER.to_vec();
    while let (Some(size), Some(ty)) = (rest.get(..4).and_then(|s| <[u8; 4]>::try_from(s).ok()), rest.get(4..8)) {
        let (head, len) = match u32::from_be_bytes(size) {
            0 => (8, rest.len() as u64),
            1 => match rest.get(8..16).and_then(|l| l.try_into().ok()) {
                Some(l) => (16, u64::from_be_bytes(l)),
                None => break,
            },
            n => (8, u64::from(n)),
        };
        if len < head {
            break;
        }
        let len = usize::try_from(len).unwrap_or(usize::MAX).min(rest.len());
        let (b, after) = rest.split_at(len);
        if matches!(ty, b"ftyp" | b"jxll" | b"jxlc" | b"jxlp") {
            if (out.len() as u64).saturating_add(b.len() as u64) > max_copy {
                return Err("the JPEG XL container is too large to read".into());
            }
            out.try_reserve_exact(b.len()).map_err(|_| "the JPEG XL container is too large to read")?;
            out.extend_from_slice(b);
        }
        rest = after;
    }
    Ok(Some(out))
}

fn too_large(w: u32, h: u32) -> String {
    format!("an image of {w} × {h} pixels is too large to draw")
}

fn unreadable(e: impl std::fmt::Display) -> String {
    format!("the JPEG XL file can't be read: {e}")
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}
