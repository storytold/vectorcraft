//! HEIC and HEIF images for File › Open and File › Place, decoded with `craft-heif` when the
//! engine's `heif` feature is on (the default). The image is bounded before decoding and converted
//! to RGBA for the common PNG storage path. A build without the feature still recognizes the files
//! and refuses them with a message.

#[cfg(feature = "heif")]
pub(super) use decoder::{decode, quarter_turned, with_exif};

/// The brands `craft-heif` reads: HEVC-coded images and image sequences (ISO/IEC 23008-12 annex
/// B) and the general image and sequence brands.
const BRANDS: [&[u8; 4]; 7] = [b"heic", b"heix", b"heim", b"heis", b"hevc", b"mif1", b"msf1"];

/// Recognize a HEIF file by its `ftyp` box: one of [`BRANDS`] as the major brand or a compatible
/// one, and no AVIF brand (`avif`, `avis`). The box is read as JP2 boxes are
/// ([`super::jp2read::boxes`]): ISO BMFF sizes, the 64-bit one included.
pub(super) fn is_heif(bytes: &[u8]) -> bool {
    let Some((b"ftyp", ftyp)) = super::jp2read::boxes(bytes).next() else { return false };
    let Some(major) = ftyp.get(..4).and_then(|b| <&[u8; 4]>::try_from(b).ok()) else { return false };
    if ftyp.len() < 8 {
        return false;
    }
    let compatible = ftyp.get(8..).unwrap_or_default().as_chunks::<4>().0;
    let brands = || std::iter::once(major).chain(compatible);
    !brands().any(|brand| matches!(brand, b"avif" | b"avis")) && brands().any(|brand| BRANDS.contains(&brand))
}

/// Without the `heif` feature every HEIF file is refused.
#[cfg(not(feature = "heif"))]
pub(super) fn decode(_bytes: &[u8], _max_alloc: u64) -> Result<image::RgbaImage, String> {
    Err("HEIC and HEIF images can't be opened in this build".into())
}

/// Without the `heif` feature no image is decoded, so none is turned.
#[cfg(not(feature = "heif"))]
pub(super) fn quarter_turned(_bytes: &[u8]) -> bool {
    false
}

/// Without the `heif` feature no resolution is read.
#[cfg(not(feature = "heif"))]
pub(super) fn with_exif<T>(_bytes: &[u8], _read: impl FnOnce(&[u8]) -> Option<T>) -> Option<T> {
    None
}

#[cfg(feature = "heif")]
mod decoder {
    use craft_heif::{Colour, Error, Options};

    /// The message for a file that isn't a readable HEIF image.
    const UNREADABLE: &str = "the HEIC or HEIF file can't be read";
    /// The message for an image kind that `craft-heif` does not handle.
    const UNSUPPORTED: &str = "the HEIC or HEIF file holds a kind of image that can't be read";

    /// Decode the primary image within `max_alloc` bytes, with the container's rotation, mirror,
    /// and crop applied. An embedded RGB ICC profile, or `nclx` Display P3 primaries with the sRGB
    /// curve, is converted to sRGB; other `nclx` values are read as sRGB.
    pub(in super::super) fn decode(bytes: &[u8], max_alloc: u64) -> Result<image::RgbaImage, String> {
        let refused = |e: Error| match e {
            Error::Unsupported(_) => UNSUPPORTED.to_string(),
            _ => UNREADABLE.to_string(),
        };
        let info = craft_heif::probe(bytes).map_err(refused)?;
        let too_large = || format!("an image of {} × {} pixels is too large to draw", info.coded_width, info.coded_height);
        let coded = u64::from(info.coded_width).checked_mul(u64::from(info.coded_height)).ok_or_else(too_large)?;
        let shown = u64::from(info.width).checked_mul(u64::from(info.height)).ok_or_else(too_large)?;
        if coded == 0 || shown == 0 || coded.checked_mul(17).is_none_or(|n| n > max_alloc) {
            return Err(too_large());
        }
        let decoded = craft_heif::decode(bytes, &Options { max_pixels: max_alloc / 17, ..Options::default() }).map_err(|e| match e {
            Error::Limit(_) => too_large(),
            e => refused(e),
        })?;
        let channels = if decoded.has_alpha { 4usize } else { 3usize };
        let profile = source_profile(decoded.icc.as_deref(), decoded.colour);
        let to_u8 = |s: u16| ((u32::from(s) * 255 + 32767) / 65535) as u8;
        let mut rgba = Vec::with_capacity(decoded.width as usize * decoded.height as usize * 4);
        for px in decoded.data.chunks_exact(channels) {
            let a = if decoded.has_alpha { to_u8(px[3]) } else { 255 };
            let [r, g, b] = if let Some(profile) = profile.as_ref() {
                let device = [px[0], px[1], px[2]].map(|v| f32::from(v) / 65535.0);
                profile
                    .to_srgb_into(&device, vectorcraft_color::cms::Intent::RelativeColorimetric)
                    .map(|rgb| rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
                    .unwrap_or_else(|| [to_u8(px[0]), to_u8(px[1]), to_u8(px[2])])
            } else {
                [to_u8(px[0]), to_u8(px[1]), to_u8(px[2])]
            };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        image::RgbaImage::from_raw(decoded.width, decoded.height, rgba).ok_or_else(|| UNREADABLE.into())
    }

    /// Whether the container's transforms swap width and height.
    pub(in super::super) fn quarter_turned(bytes: &[u8]) -> bool {
        craft_heif::probe(bytes).ok().is_some_and(|i| (i.width > i.height) != (i.coded_width > i.coded_height))
    }

    /// `read` of the primary image's EXIF block, a TIFF structure inside `bytes`, for its
    /// resolution tags.
    pub(in super::super) fn with_exif<T>(bytes: &[u8], read: impl FnOnce(&[u8]) -> Option<T>) -> Option<T> {
        let (exif, _) = craft_heif::read_metadata(bytes).ok()?;
        exif.as_deref().and_then(read)
    }

    const P3_PRIMARIES: u16 = 12;
    const SRGB_TRANSFER: u16 = 13;

    fn source_profile(icc: Option<&[u8]>, colour: Colour) -> Option<vectorcraft_color::cms::IccProfile> {
        use vectorcraft_color::cms::{self, IccProfile, ProfileKind};
        if let Some(icc) = icc {
            return IccProfile::from_bytes(None, icc).ok().filter(|p| p.kind == ProfileKind::Rgb);
        }
        (colour.primaries == P3_PRIMARIES && colour.transfer == SRGB_TRANSFER).then(|| cms::icc::builtin_rgb(cms::DISPLAY_P3)).flatten()
    }
}
