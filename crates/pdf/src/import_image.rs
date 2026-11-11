//! CMYK images keep their samples, as CMYK colours do: DeviceCMYK images and ICC-based ones with
//! four components. A JPEG whose Decode array inverts its samples (Adobe's CMYK JPEGs, the
//! convention JPEG readers and the PDF writer take a CMYK JPEG to follow) is kept as it is; any
//! other is decoded, its Decode array applied, and stored as a CMYK TIFF
//! ([`ImageBlob::cmyk_tiff`]). A mask (`/SMask`, `/Mask`) doesn't change that: the importer
//! carries it as the image's opacity mask ([`has_mask`]). A DeviceN image of process plates only
//! keeps them the same way, as a CMYK image ([`process_devicen`]). An image of one ink (a
//! Separation, or a DeviceN of one colorant) keeps it: its tints are read as a grayscale image
//! ([`tint_gray`]) that the ink colorizes. Every other image is read as RGB by the interpreter.

use craft_pdf::syntax::Filter;
use craft_pdf::syntax::object::stream::{ImageColorSpace, ImageDecodeParams};
use craft_pdf::syntax::object::{Array, Dict, Name, Stream};
use vectorcraft_doc::ImageBlob;
use vectorcraft_doc::cmyk::{Inks, MAX_CMYK_PIXELS, jpeg_components};

use crate::import_color::ink_names;

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
        return process_devicen(st, width, height);
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

/// An image in a spot ink read as RGB: why.
pub(crate) const UNREAD_TINTS: &str = "images in a spot ink whose samples couldn't be read were imported in RGB";

/// The image's colour space when it lays one ink: a Separation (`[/Separation name alt tint]`), or
/// a DeviceN of one colorant (`[/DeviceN [name] alt tint …]`). `None` for the ink `None`, which
/// paints nothing (ISO 32000-1 §8.6.6.4): the interpreter draws it.
pub(crate) fn one_ink_space<'a>(st: &Stream<'a>) -> Option<Array<'a>> {
    let d = st.dict();
    let a = d.get::<Array<'a>>(key(d, b"ColorSpace", b"CS"))?;
    let one_ink = matches!(ink_names(&mut a.flex_iter()).as_ref().map(|(_, names)| names.as_slice()), Some([name]) if name != "None");
    one_ink.then_some(a)
}

/// The CMYK channel of each colorant of the image's DeviceN space (`None` for the ink `None`) when
/// it has two or more colorants and all are process plates, or `None`. One colorant is an image of
/// one ink ([`one_ink_space`]).
fn process_plates(d: &Dict<'_>) -> Option<Vec<Option<usize>>> {
    let a = d.get::<Array<'_>>(key(d, b"ColorSpace", b"CS"))?;
    let (device_n, names) = ink_names(&mut a.flex_iter())?;
    // PDF's own limit is 32 colorants.
    if !device_n || names.len() < 2 || names.len() > 32 {
        return None;
    }
    let plates = names
        .iter()
        .map(|n| match n.as_str() {
            "None" => Some(None),
            n => ["Cyan", "Magenta", "Yellow", "Black"].iter().position(|p| *p == n).map(Some),
        })
        .collect::<Option<Vec<_>>>()?;
    plates.iter().any(Option::is_some).then_some(plates)
}

/// A DeviceN image of process plates only ([`process_plates`]) in CMYK: each colorant's tint in
/// its plate, its Decode array applied, as a CMYK TIFF. `Ok(None)` for any other image, `Err` (a
/// warning) when its samples can't be read.
fn process_devicen(st: &Stream<'_>, width: u32, height: u32) -> Result<Option<ImageBlob>, &'static str> {
    let d = st.dict();
    let Some(plates) = process_plates(d) else { return Ok(None) };
    let bpc = d.get::<u8>(key(d, b"BitsPerComponent", b"BPC")).unwrap_or(8);
    if !matches!(bpc, 8 | 16) {
        return Err(UNREAD);
    }
    let n = u64::from(width) * u64::from(height);
    if n == 0 || n > MAX_CMYK_PIXELS {
        return Err(UNREAD);
    }
    let comps = plates.len();
    let ranges: Vec<(f32, f32)> = match d.get::<Array<'_>>(key(d, b"Decode", b"D")).map(|a| a.iter::<f32>().collect::<Vec<_>>()) {
        Some(v) if v.len() == 2 * comps && v.iter().all(|x| x.is_finite()) => v.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect(),
        _ => vec![(0.0, 1.0); comps],
    };
    let size = usize::from(bpc / 8);
    let params =
        ImageDecodeParams { is_indexed: false, bpc: Some(bpc), num_components: u8::try_from(comps).ok(), target_dimension: None, width, height };
    let decoded = st.decoded_image(&params).map_err(|_| UNREAD)?;
    // An image codec lays its samples out its own way.
    if decoded.image_data.is_some() {
        return Err(UNREAD);
    }
    // Within `MAX_CMYK_PIXELS` and 32 colorants of 2 bytes this fits a 64-bit `usize`, not always a
    // 32-bit one (the web build): checked.
    let len = usize::try_from(n).ok().and_then(|n| n.checked_mul(comps)?.checked_mul(size)).ok_or(UNREAD)?;
    let raw = decoded.data.get(..len).ok_or(UNREAD)?;
    let value = |s: &[u8]| match *s {
        [hi, lo] => u16::from_be_bytes([hi, lo]) as f32 / 65535.0,
        [v] => v as f32 / 255.0,
        _ => 0.0,
    };
    let mut inks = Vec::with_capacity(n as usize * 4);
    for px in raw.chunks_exact(size * comps) {
        let mut cmyk = [0.0f32; 4];
        for ((s, plate), (lo, hi)) in px.chunks_exact(size).zip(&plates).zip(&ranges) {
            if let Some(c) = plate.and_then(|p| cmyk.get_mut(p)) {
                *c = c.max((lo + value(s) * (hi - lo)).clamp(0.0, 1.0));
            }
        }
        inks.extend(cmyk.map(|v| (v * 255.0).round() as u8));
    }
    Inks::new(width, height, inks).and_then(|i| ImageBlob::cmyk_tiff(&i)).map(Some).ok_or(UNREAD)
}

/// The tints of the one-component image `st` (`width` × `height` pixels, an image of one ink)
/// as a grayscale image: white where it lays no ink, black at 100 %, its Decode array applied.
/// `Err` (a warning) when its samples can't be read.
pub(crate) fn tint_gray(st: &Stream<'_>, width: u32, height: u32) -> Result<Vec<u8>, &'static str> {
    let d = st.dict();
    let bpc = d.get::<u8>(key(d, b"BitsPerComponent", b"BPC")).unwrap_or(8);
    if !matches!(bpc, 1 | 2 | 4 | 8 | 16) {
        return Err(UNREAD_TINTS);
    }
    // The same cap as CMYK images'.
    let n = u64::from(width) * u64::from(height);
    if n == 0 || n > MAX_CMYK_PIXELS {
        return Err(UNREAD_TINTS);
    }
    let (lo, hi) = match d.get::<Array<'_>>(key(d, b"Decode", b"D")).map(|a| a.iter::<f32>().collect::<Vec<_>>()).as_deref() {
        Some(&[lo, hi]) if lo.is_finite() && hi.is_finite() => (lo, hi),
        _ => (0.0, 1.0),
    };
    let params = ImageDecodeParams { is_indexed: false, bpc: Some(bpc), num_components: Some(1), target_dimension: None, width, height };
    let decoded = st.decoded_image(&params).map_err(|_| UNREAD_TINTS)?;
    // An image codec (JPEG, CCITT, JBIG2, JPEG 2000) says what it decoded to: one gray channel of
    // the image's size, at its own bit depth.
    let bpc = match &decoded.image_data {
        None => bpc,
        Some(i)
            if i.alpha.is_none()
                && matches!(i.color_space, Some(ImageColorSpace::Gray))
                && (i.width, i.height) == (width, height)
                && matches!(i.bits_per_component, 1 | 2 | 4 | 8 | 16) =>
        {
            i.bits_per_component
        }
        Some(_) => return Err(UNREAD_TINTS),
    };
    let bits = usize::from(bpc);
    let row = (width as usize * bits).div_ceil(8);
    let data = decoded.data.get(..row * height as usize).ok_or(UNREAD_TINTS)?;
    let max = ((1u32 << bits) - 1) as f32;
    let mut out = Vec::with_capacity(n as usize);
    for line in data.chunks_exact(row) {
        for x in 0..width as usize {
            let v = match bits {
                16 => line.get(x * 2..x * 2 + 2).map_or(0, |s| s.iter().fold(0u32, |v, b| (v << 8) | u32::from(*b))),
                8 => line.get(x).map_or(0, |b| u32::from(*b)),
                // 1, 2 or 4 bits a sample, packed from the high bits of each byte.
                _ => line.get(x * bits / 8).map_or(0, |b| (u32::from(*b) >> (8 - bits - (x * bits) % 8)) & ((1 << bits) - 1)),
            };
            let tint = (lo + v as f32 / max * (hi - lo)).clamp(0.0, 1.0);
            out.push(((1.0 - tint) * 255.0).round() as u8);
        }
    }
    Ok(out)
}

/// Does the image carry its own transparency (a soft mask, a stencil mask or a colour key)?
pub(crate) fn has_mask(st: &Stream<'_>) -> bool {
    let d = st.dict();
    d.contains_key(b"SMask") || d.contains_key(b"Mask") || d.get::<u8>(b"SMaskInData").is_some_and(|v| v != 0)
}
