//! AVIF images (`.avif`, the AV1 Image File Format) for File › Open and File › Place, decoded by
//! craft-image-codecs's `avif-decode` (craft-video's clean-room, pure-Rust AV1 decoder) when the
//! engine's `avif` feature is on (the default) and the build is native. The image is bounded
//! before decoding and converted to 8-bit sRGB RGBA for the common PNG storage path, as HEIC and
//! HEIF images are ([`super::heifread`]).
//!
//! The decoder reads the primary item as one `av01` picture or a `grid` of them, at 8, 10 or 12
//! bits, with an alpha auxiliary image, and applies the container's `clap`, `irot` and `imir`.
//! Colors become sRGB: an embedded RGB ICC profile is converted by the color engine, and `nclx`
//! Display P3 or BT.2020 primaries by their matrices; other `nclx` values are read as sRGB, as for
//! HEIF. PQ and HLG images, which the decoder gives as linear light, are clipped at SDR reference
//! white. The EXIF resolution tags size the image, their axes swapped when the container turns it
//! an odd number of quarter turns. An image sequence (animated AVIF) opens as its primary still
//! image, the first frame libavif writes; one without a still image, layered images and builds
//! without the decoder (the web build among them: a panic on wasm32 aborts instead of unwinding)
//! refuse the file with a message.

/// Brands that name an AVIF file: still images (`avif`) and image sequences (`avis`), AV1 Image
/// File Format v1.1.0 §8.
const BRANDS: [&[u8; 4]; 2] = [b"avif", b"avis"];

/// The message for a file that isn't a readable AVIF image.
#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
const UNREADABLE: &str = "the AVIF file can't be read";
/// The message for an image sequence, a layered image or another AV1 coding the decoder doesn't
/// read.
#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
const UNSUPPORTED: &str = "the AVIF file holds a kind of image that can't be read";

/// Recognize an AVIF file by its `ftyp` box: an AVIF brand ([`BRANDS`]) as the major brand or a
/// compatible one. [`super::heifread::is_heif`] refuses the same files, so the two never overlap.
pub(super) fn is_avif(bytes: &[u8]) -> bool {
    let Some((b"ftyp", ftyp)) = super::jp2read::boxes(bytes).next() else { return false };
    let Some(major) = ftyp.get(..4).and_then(|b| <&[u8; 4]>::try_from(b).ok()) else { return false };
    if ftyp.len() < 8 {
        return false;
    }
    let compatible = ftyp.get(8..).unwrap_or_default().as_chunks::<4>().0;
    std::iter::once(major).chain(compatible).any(|brand| BRANDS.contains(&brand))
}

/// Without the decoder every AVIF file is refused.
#[cfg(not(all(feature = "avif", not(target_arch = "wasm32"))))]
pub(super) fn decode(_bytes: &[u8], _max_alloc: u64) -> Result<(image::RgbaImage, Option<(f64, f64)>), String> {
    Err("AVIF images can't be opened in this build".into())
}

/// The primary image as straight sRGB RGBA, decoded within `max_alloc` bytes, with the container's
/// crop, rotation and mirror applied, and the resolution its EXIF tags give the shown pixels. A
/// panic inside the decoder is reported as an unreadable file.
#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
pub(super) fn decode(bytes: &[u8], max_alloc: u64) -> Result<(image::RgbaImage, Option<(f64, f64)>), String> {
    crate::guard::catch_panic(|| decoder::decode(bytes, max_alloc)).unwrap_or_else(|_| Err(UNREADABLE.into()))
}

/// Whether the container's `irot` properties of the primary item add up to an odd number of
/// quarter turns, so the shown image's width and height are the stored image's swapped. A
/// container that can't be read is not turned.
#[cfg(any(test, all(feature = "avif", not(target_arch = "wasm32"))))]
pub(super) fn quarter_turned(bytes: &[u8]) -> bool {
    use super::jp2read::boxes;
    let turned = || -> Option<bool> {
        let (_, meta) = boxes(bytes).find(|(t, _)| *t == b"meta")?;
        // `meta` is a full box: its version and flags come before its children.
        let children = meta.get(4..)?;
        let (_, pitm) = boxes(children).find(|(t, _)| *t == b"pitm")?;
        let primary = match pitm.first()? {
            0 => u32::from(u16::from_be_bytes(pitm.get(4..6)?.try_into().ok()?)),
            _ => u32::from_be_bytes(pitm.get(4..8)?.try_into().ok()?),
        };
        let (_, iprp) = boxes(children).find(|(t, _)| *t == b"iprp")?;
        let (_, ipco) = boxes(iprp).find(|(t, _)| *t == b"ipco")?;
        let properties: Vec<_> = boxes(ipco).collect();
        let mut quarters = 0u32;
        for (_, ipma) in boxes(iprp).filter(|(t, _)| *t == b"ipma") {
            let (version, wide_index) = (*ipma.first()?, ipma.get(3)? & 1 == 1);
            let count = u32::from_be_bytes(ipma.get(4..8)?.try_into().ok()?);
            let mut at = 8usize;
            for _ in 0..count {
                let id = if version < 1 {
                    let id = u16::from_be_bytes(ipma.get(at..at + 2)?.try_into().ok()?);
                    at += 2;
                    u32::from(id)
                } else {
                    let id = u32::from_be_bytes(ipma.get(at..at + 4)?.try_into().ok()?);
                    at += 4;
                    id
                };
                let associations = *ipma.get(at)?;
                at += 1;
                for _ in 0..associations {
                    let index = if wide_index {
                        let v = u16::from_be_bytes(ipma.get(at..at + 2)?.try_into().ok()?) & 0x7fff;
                        at += 2;
                        usize::from(v)
                    } else {
                        let v = *ipma.get(at)? & 0x7f;
                        at += 1;
                        usize::from(v)
                    };
                    // Index 0 means no property; the others count from 1.
                    if id != primary || index == 0 {
                        continue;
                    }
                    if let Some((b"irot", irot)) = properties.get(index - 1).map(|(t, d)| (*t, *d)) {
                        quarters += u32::from(irot.first()? & 3);
                    }
                }
            }
        }
        Some(quarters % 2 == 1)
    };
    turned().unwrap_or(false)
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
mod decoder {
    use craft_image_codecs::{ChannelLayout, Cicp, CodecError, DecodeOptions, Format, Limits, SampleType};
    use vectorcraft_color::cms::{IccProfile, Intent, ProfileKind};

    use super::{UNREADABLE, UNSUPPORTED};

    /// `nclx` color primaries, ITU-T H.273: BT.2020 and Display P3 (SMPTE EG 432-1, D65). BT.709
    /// (sRGB) primaries, and the ones not named here, are read as sRGB.
    const BT2020_PRIMARIES: u16 = 9;
    const P3_PRIMARIES: u16 = 12;

    /// Linear BT.2020 RGB to linear BT.709 RGB (both D65).
    const BT2020_TO_709: [[f32; 3]; 3] =
        [[1.660_491, -0.587_641_1, -0.072_849_9], [-0.124_550_5, 1.132_899_9, -0.008_349_4], [-0.018_150_8, -0.100_578_9, 1.118_729_7]];
    /// Linear Display P3 RGB to linear BT.709 RGB (both D65).
    const P3_TO_709: [[f32; 3]; 3] = [[1.224_940_2, -0.224_940_2, 0.0], [-0.042_056_9, 1.042_056_9, 0.0], [-0.019_637_6, -0.078_636_1, 1.098_273_7]];

    pub(super) fn decode(bytes: &[u8], max_alloc: u64) -> Result<(image::RgbaImage, Option<(f64, f64)>), String> {
        let limits = Limits { max_alloc, max_pixels: max_alloc / 4, ..Limits::default() };
        let opts = DecodeOptions { limits, keep_orientation: false };
        let image = craft_image_codecs::decode_as_with(Format::Avif, bytes, &opts).map_err(|e| match e {
            CodecError::LimitExceeded(_) => "an image is too large to draw".to_string(),
            CodecError::Unsupported { .. } => UNSUPPORTED.to_string(),
            _ => UNREADABLE.to_string(),
        })?;
        let (w, h) = (image.width(), image.height());
        let too_large = || format!("an image of {w} × {h} pixels is too large to draw");
        let pixels = u64::from(w).checked_mul(u64::from(h)).ok_or_else(too_large)?;
        let rgba_size = pixels.checked_mul(4).ok_or_else(too_large)?;
        if pixels == 0 || rgba_size > max_alloc {
            return Err(too_large());
        }
        let has_alpha = match image.layout() {
            ChannelLayout::Rgb => false,
            ChannelLayout::Rgba => true,
            _ => return Err(UNSUPPORTED.into()),
        };
        let stride = if has_alpha { 4 } else { 3 };
        let sample = image.sample_type();
        let width = match sample {
            SampleType::U8 => 1,
            SampleType::U16 => 2,
            SampleType::F32 => 4,
            _ => return Err(UNSUPPORTED.into()),
        };
        let cicp = image.cicp;
        // The decoder gives PQ and HLG images as linear light, 1.0 at SDR reference white.
        let linear = cicp.is_some_and(|c| c.transfer == Cicp::TRANSFER_LINEAR);
        let matrix = cicp.and_then(|c| match c.primaries {
            BT2020_PRIMARIES => Some(BT2020_TO_709),
            P3_PRIMARIES => Some(P3_TO_709),
            _ => None,
        });
        // An ICC profile describes non-linear samples; it isn't read for linear ones.
        let profile =
            image.icc.as_deref().filter(|_| !linear).and_then(|icc| IccProfile::from_bytes(None, icc).ok()).filter(|p| p.kind == ProfileKind::Rgb);
        let read = |s: &[u8]| -> f32 {
            match sample {
                SampleType::U8 => f32::from(s.first().copied().unwrap_or(0)) / 255.0,
                SampleType::U16 => f32::from(u16::from_ne_bytes([s.first().copied().unwrap_or(0), s.get(1).copied().unwrap_or(0)])) / 65535.0,
                _ => s.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map_or(0.0, f32::from_ne_bytes),
            }
        };
        let mut rgba = Vec::with_capacity(usize::try_from(rgba_size).map_err(|_| too_large())?);
        let data = image.data();
        let pixel_bytes = stride * width;
        if data.len() < usize::try_from(pixels).map_err(|_| too_large())?.saturating_mul(pixel_bytes) {
            return Err(UNREADABLE.into());
        }
        for px in data.chunks_exact(pixel_bytes) {
            let at = |i: usize| read(px.get(i * width..).unwrap_or_default());
            let device = [at(0), at(1), at(2)];
            let rgb = if let Some(profile) = profile.as_ref() {
                profile.to_srgb_into(&device, Intent::RelativeColorimetric).unwrap_or(device)
            } else if linear {
                let lin = matrix.map_or(device, |m| mul(m, device));
                lin.map(encode_srgb)
            } else if let Some(m) = matrix {
                mul(m, device.map(decode_srgb)).map(encode_srgb)
            } else {
                device
            };
            let [r, g, b] = rgb.map(to_u8);
            let a = if has_alpha { to_u8(at(3)) } else { 255 };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        let ppi = image.meta.exif.as_deref().and_then(vectorcraft_doc::ppi::tiff);
        let ppi = ppi.map(|(x, y)| if super::quarter_turned(bytes) { (y, x) } else { (x, y) });
        let img = image::RgbaImage::from_raw(w, h, rgba).ok_or(UNREADABLE)?;
        Ok((img, ppi))
    }

    fn mul(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
        m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
    }

    /// The sRGB curve (IEC 61966-2-1), encoded value to linear light.
    fn decode_srgb(v: f32) -> f32 {
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    }

    /// Linear light to the sRGB curve, clipped to the SDR range.
    fn encode_srgb(v: f32) -> f32 {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    }

    fn to_u8(v: f32) -> u8 {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        (v * 255.0).round() as u8
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn matrices_keep_white_white() {
            for m in [BT2020_TO_709, P3_TO_709] {
                for c in mul(m, [1.0; 3]) {
                    assert!((c - 1.0).abs() < 1e-4, "{c}");
                }
            }
        }

        #[test]
        fn srgb_curve_round_trips() {
            for i in 0..=255u8 {
                let v = f32::from(i) / 255.0;
                assert_eq!(to_u8(encode_srgb(decode_srgb(v))), i);
            }
        }
    }
}
