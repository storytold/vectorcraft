//! CMYK images keep their samples, as CMYK colours do: DeviceCMYK images and ICC-based ones with
//! four components. A JPEG whose Decode array inverts its samples (Adobe's CMYK JPEGs, the
//! convention JPEG readers and the PDF writer take a CMYK JPEG to follow) is kept as it is; any
//! other is decoded, its Decode array applied, and stored as a CMYK TIFF
//! ([`ImageBlob::cmyk_tiff`]). A mask (`/SMask`, `/Mask`) doesn't change that: the importer
//! carries it as the image's opacity mask ([`has_mask`]). Every other image is read as RGB by the
//! interpreter.

use craft_pdf::syntax::Filter;
use craft_pdf::syntax::object::stream::ImageDecodeParams;
use craft_pdf::syntax::object::{Array, Dict, Name, Stream};
use vectorcraft_doc::ImageBlob;
use vectorcraft_doc::cmyk::{Inks, MAX_CMYK_PIXELS, jpeg_components};

/// A CMYK image read as RGB: why.
pub(crate) const UNREAD: &str = "CMYK images whose samples couldn't be read in CMYK were imported in RGB";

/// The Decode array of a JPEG kept as it is.
const INVERTED: [(f32, f32); 4] = [(1.0, 0.0); 4];

/// `key`, or its inline-image abbreviation `short` when `d` doesn't have it.
fn key<'k>(d: &Dict<'_>, key: &'k [u8], short: &'k [u8]) -> &'k [u8] {
    if d.contains_key(key) { key } else { short }
}

/// Are the image's colours CMYK (DeviceCMYK, CalCMYK, or ICC-based with four components)?
fn cmyk_space(d: &Dict<'_>) -> bool {
    let cs = key(d, b"ColorSpace", b"CS");
    if let Some(n) = d.get::<Name<'_>>(cs) {
        return matches!(n.as_ref(), b"DeviceCMYK" | b"CMYK");
    }
    let Some(a) = d.get::<Array<'_>>(cs) else { return false };
    let mut it = a.flex_iter();
    match it.next::<Name<'_>>() {
        Some(n) if n.as_ref() == b"CalCMYK" => true,
        Some(n) if n.as_ref() == b"ICCBased" => it.next::<Stream<'_>>().and_then(|s| s.dict().get::<u8>(b"N")) == Some(4),
        _ => false,
    }
}

/// Each component's Decode range (`[0 1]` each when the array is missing or malformed).
fn decode_ranges(d: &Dict<'_>) -> [(f32, f32); 4] {
    let mut out = [(0.0, 1.0); 4];
    let Some(a) = d.get::<Array<'_>>(key(d, b"Decode", b"D")) else { return out };
    let v: Vec<f32> = a.iter::<f32>().collect();
    if v.len() == 8 && v.iter().all(|x| x.is_finite()) {
        for (o, p) in out.iter_mut().zip(v.as_chunks::<2>().0) {
            *o = (p[0], p[1]);
        }
    }
    out
}

/// The image `st` (`width` × `height` pixels) in CMYK: `Ok(None)` when it isn't a CMYK image,
/// `Err` (a warning) when it is one that is read as RGB.
pub(crate) fn cmyk(st: &Stream<'_>, width: u32, height: u32) -> Result<Option<ImageBlob>, &'static str> {
    let d = st.dict();
    if !cmyk_space(d) {
        return Ok(None);
    }
    let ranges = decode_ranges(d);
    let filters = st.filters();
    if ranges == INVERTED && matches!(filters.as_slice(), [Filter::DctDecode]) {
        let raw = st.raw_data();
        if jpeg_components(&raw) == Some(4) {
            return Ok(Some(ImageBlob::new("image/jpeg", raw.into_owned())));
        }
    }
    let bpc = d.get::<u8>(key(d, b"BitsPerComponent", b"BPC")).unwrap_or(8);
    if !matches!(bpc, 8 | 16) {
        return Err(UNREAD);
    }
    let n = width as u64 * height as u64;
    if n == 0 || n > MAX_CMYK_PIXELS {
        return Err(UNREAD);
    }
    let size = usize::from(bpc / 8);
    let params = ImageDecodeParams { is_indexed: false, bpc: Some(bpc), num_components: Some(4), target_dimension: None, width, height };
    let Ok(decoded) = st.decoded_image(&params) else { return Err(UNREAD) };
    let Some(raw) = decoded.data.get(..n as usize * 4 * size) else { return Err(UNREAD) };
    let value = |s: &[u8]| match *s {
        [hi, lo] => u16::from_be_bytes([hi, lo]) as f32 / 65535.0,
        [v] => v as f32 / 255.0,
        _ => 0.0,
    };
    let inks = raw
        .chunks_exact(size * 4)
        .flat_map(|px| px.chunks_exact(size).zip(ranges).map(|(s, (lo, hi))| ((lo + value(s) * (hi - lo)).clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    Inks::new(width, height, inks).and_then(|i| ImageBlob::cmyk_tiff(&i)).map(Some).ok_or(UNREAD)
}

/// Does the image carry its own transparency (a soft mask, a stencil mask or a colour key)?
pub(crate) fn has_mask(st: &Stream<'_>) -> bool {
    let d = st.dict();
    d.contains_key(b"SMask") || d.contains_key(b"Mask") || d.get::<u8>(b"SMaskInData").is_some_and(|v| v != 0)
}
