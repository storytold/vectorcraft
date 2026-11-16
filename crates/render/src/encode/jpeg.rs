//! JPEG export: RGB, CMYK or greyscale pixels, baseline, optimized or progressive (3–5 scans), the
//! resolution in the JFIF header and the colour profile in APP2 segments. Profiles come from the
//! CMS ([`cms::icc_bytes`]): sRGB for RGB (the renderer's pixels are sRGB), the working CMYK space
//! for CMYK, [`cms::GRAY`] for greyscale.

use std::collections::HashMap;

use jpeg_encoder::{ColorType, Density, Encoder};
use vectorcraft_color::cms::{self, lab};

use crate::Rendered;

/// The colour model a JPEG is written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorModel {
    #[default]
    Rgb,
    /// Ink amounts in the working CMYK space.
    Cmyk,
    /// Grey with the lightness of the colour.
    Gray,
}

impl ColorModel {
    pub const ALL: [ColorModel; 3] = [ColorModel::Rgb, ColorModel::Cmyk, ColorModel::Gray];

    pub fn id(self) -> &'static str {
        match self {
            ColorModel::Rgb => "rgb",
            ColorModel::Cmyk => "cmyk",
            ColorModel::Gray => "gray",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorModel::Rgb => "RGB",
            ColorModel::Cmyk => "CMYK",
            ColorModel::Gray => "Grayscale",
        }
    }

    /// By id or label, ignoring case (`grey` and `grayscale` are grey too).
    pub fn from_id(s: &str) -> Option<Self> {
        let s = s.to_ascii_lowercase();
        Self::ALL.into_iter().find(|m| m.id() == s || m.label().eq_ignore_ascii_case(&s)).or_else(|| (s == "grey").then_some(ColorModel::Gray))
    }

    /// Bytes per pixel of the pixels [`encode`] takes.
    pub fn channels(self) -> usize {
        match self {
            ColorModel::Rgb => 3,
            ColorModel::Cmyk => 4,
            ColorModel::Gray => 1,
        }
    }

    /// The profile that describes these pixels.
    pub(crate) fn profile(self) -> String {
        match self {
            ColorModel::Rgb => cms::SRGB.into(),
            ColorModel::Cmyk => cms::active_settings().cmyk,
            ColorModel::Gray => cms::GRAY.into(),
        }
    }
}

/// How the image data is coded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Method {
    /// Standard Huffman tables: every decoder reads it.
    #[default]
    Baseline,
    /// Huffman tables optimized for the image: smaller files.
    Optimized,
    /// Several scans, each adding detail: the image builds up while it loads.
    Progressive,
}

impl Method {
    pub const ALL: [Method; 3] = [Method::Baseline, Method::Optimized, Method::Progressive];

    pub fn id(self) -> &'static str {
        match self {
            Method::Baseline => "baseline",
            Method::Optimized => "optimized",
            Method::Progressive => "progressive",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Method::Baseline => "Baseline (Standard)",
            Method::Optimized => "Baseline Optimized",
            Method::Progressive => "Progressive",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(s) || m.label().eq_ignore_ascii_case(s))
    }
}

/// Progressive scans: 3 to 5.
pub const SCANS: std::ops::RangeInclusive<u8> = 3..=5;

/// The JPEG options of a raster export.
#[derive(Clone, Debug, PartialEq)]
pub struct JpegOptions {
    pub color_model: ColorModel,
    pub method: Method,
    /// Progressive scans ([`SCANS`]).
    pub scans: u8,
    /// Embed the colour profile.
    pub embed_icc: bool,
}

impl Default for JpegOptions {
    fn default() -> Self {
        Self { color_model: ColorModel::Rgb, method: Method::Baseline, scans: 3, embed_icc: true }
    }
}

/// Encode `px` (`o.color_model`'s channels per pixel: RGB, CMYK ink amounts with 0 = no ink, or
/// grey) at `quality` 0–100, with the resolution `ppi` (if any) in the JFIF header.
pub fn encode(px: &[u8], width: u32, height: u32, quality: u8, ppi: Option<f64>, o: &JpegOptions) -> Result<Vec<u8>, String> {
    let fail = |e: jpeg_encoder::EncodingError| format!("JPEG encoding failed: {e}");
    let (Ok(w), Ok(h)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err(format!("{width} × {height} pixels is too large for JPEG (at most 65535 pixels a side): lower the resolution"));
    };
    if px.len() as u64 != u64::from(width) * u64::from(height) * o.color_model.channels() as u64 {
        return Err("JPEG encoding failed: pixel buffer doesn't match the image size".into());
    }
    let mut buf = Vec::new();
    let mut enc = Encoder::new(&mut buf, quality.clamp(1, 100));
    if let Some(ppi) = ppi {
        let d = ppi.round().clamp(1.0, u16::MAX as f64) as u16;
        enc.set_density(Density::Inch { x: d, y: d });
    }
    match o.method {
        Method::Baseline => {}
        Method::Optimized => enc.set_optimized_huffman_tables(true),
        Method::Progressive => {
            enc.set_optimized_huffman_tables(true);
            enc.set_progressive_scans(o.scans.clamp(*SCANS.start(), *SCANS.end()));
        }
    }
    if o.embed_icc {
        let icc = cms::icc_bytes(&o.color_model.profile()).map_err(|e| format!("JPEG colour profile: {e}"))?;
        enc.add_icc_profile(&icc).map_err(fail)?;
    }
    let ty = match o.color_model {
        ColorModel::Rgb => ColorType::Rgb,
        ColorModel::Cmyk => ColorType::Cmyk,
        ColorModel::Gray => ColorType::Luma,
    };
    enc.encode(px, w, h, ty).map_err(fail)?;
    // JFIF describes one- and three-component images (greyscale and YCbCr), so its APP0 segment
    // must not head a four-component CMYK frame: the file then says two different things about
    // its colours, and a strict reader rejects it rather than guessing. Adobe's own CMYK JPEGs
    // carry the APP14 marker the encoder already writes, and no JFIF (#3084).
    if o.color_model == ColorModel::Cmyk {
        unlabel_jfif(&mut buf, ppi);
    }
    Ok(buf)
}

/// Replace the leading APP0/JFIF segment, which the encoder always writes, with an EXIF segment
/// carrying the same resolution (TIFF `XResolution`, `YResolution` and `ResolutionUnit`, which is
/// where a reader looks when there is no JFIF). Without a resolution the segment is simply
/// dropped.
fn unlabel_jfif(buf: &mut Vec<u8>, ppi: Option<f64>) {
    // SOI, then APP0 with its two length bytes and the tag it is identified by.
    let Some(head) = buf.get(..13) else { return };
    if head[..4] != [0xFF, 0xD8, 0xFF, 0xE0] || &head[6..11] != b"JFIF\0" {
        return;
    }
    let len = usize::from(u16::from_be_bytes([head[4], head[5]]));
    if 4 + len > buf.len() {
        return;
    }
    let exif = ppi.filter(|p| p.is_finite() && *p >= 1.0).map(|p| exif_resolution(p.round() as u32)).unwrap_or_default();
    buf.splice(2..4 + len, exif);
}

/// An APP1 EXIF segment holding `dpi` as IFD0's resolution: big-endian TIFF, three entries, each
/// rational pointing past the directory.
fn exif_resolution(dpi: u32) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\0*"); // big-endian, TIFF magic
    tiff.extend_from_slice(&8u32.to_be_bytes()); // IFD0 right after the header
    tiff.extend_from_slice(&3u16.to_be_bytes()); // three entries
    // Entry: tag, type (5 = rational, 3 = short), count, then the value or its offset.
    let entry = |tag: u16, ty: u16, value: [u8; 4]| {
        let mut e = Vec::with_capacity(12);
        e.extend_from_slice(&tag.to_be_bytes());
        e.extend_from_slice(&ty.to_be_bytes());
        e.extend_from_slice(&1u32.to_be_bytes());
        e.extend_from_slice(&value);
        e
    };
    // The two rationals sit after the directory: header 8 + count 2 + 3 entries + next-IFD 4.
    let after = 8 + 2 + 3 * 12 + 4;
    tiff.extend(entry(282, 5, (after as u32).to_be_bytes())); // XResolution
    tiff.extend(entry(283, 5, (after as u32 + 8).to_be_bytes())); // YResolution
    tiff.extend(entry(296, 3, [0, 2, 0, 0])); // ResolutionUnit: inches
    tiff.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
    for _ in 0..2 {
        tiff.extend_from_slice(&dpi.to_be_bytes());
        tiff.extend_from_slice(&1u32.to_be_bytes());
    }
    let mut seg = vec![0xFF, 0xE1];
    let body_len = 6 + tiff.len() + 2;
    seg.extend_from_slice(&u16::try_from(body_len).unwrap_or(u16::MAX).to_be_bytes());
    seg.extend_from_slice(b"Exif\0\0");
    seg.extend_from_slice(&tiff);
    seg
}

/// `img` flattened on white as RGB, or as grey with each colour's lightness.
pub(crate) fn screen_pixels(img: &Rendered, gray: bool) -> Vec<u8> {
    let px = img.to_straight();
    let rgb = px.as_chunks::<4>().0.iter().map(|p| {
        let a = p[3] as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        [mix(p[0]), mix(p[1]), mix(p[2])]
    });
    if !gray {
        return rgb.flatten().collect();
    }
    rgb.map(to_gray()).collect()
}

/// A colour's grey: its relative luminance (sRGB primaries) encoded with sRGB's curve, so a grey
/// keeps its value.
pub(crate) fn to_gray() -> impl Fn([u8; 3]) -> u8 {
    let linear: [f32; 256] = std::array::from_fn(|v| lab::srgb_to_linear(v as f32 / 255.0));
    move |[r, g, b]| {
        let y = 0.2126 * linear[r as usize] + 0.7152 * linear[g as usize] + 0.0722 * linear[b as usize];
        (lab::linear_to_srgb(y).clamp(0.0, 1.0) * 255.0).round() as u8
    }
}

/// `img` flattened on white with each colour separated into the working CMYK space (ink amounts).
pub(crate) fn separated(img: &Rendered) -> Vec<u8> {
    let cms = cms::active();
    let intent = cms.settings().intent;
    let mut seen: HashMap<[u8; 3], [u8; 4]> = HashMap::new();
    screen_pixels(img, false)
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|p| {
            *seen
                .entry(*p)
                .or_insert_with(|| cms.srgb_to_cmyk(p.map(|v| v as f32 / 255.0), intent).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
        })
        .collect()
}
