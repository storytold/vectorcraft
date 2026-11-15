//! JPEG export: RGB, CMYK or greyscale pixels, baseline, optimized or progressive (3–5 scans), the
//! resolution in the JFIF header and the colour profile in APP2 segments. Profiles come from the
//! CMS ([`cms::icc_bytes`]): sRGB for RGB (the renderer's pixels are sRGB), the working CMYK space
//! for CMYK, [`cms::GRAY`] for greyscale.

use std::collections::HashMap;

use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image, JpegFormat};
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
    if u16::try_from(width).is_err() || u16::try_from(height).is_err() {
        return Err(format!("{width} × {height} pixels is too large for JPEG (at most 65535 pixels a side): lower the resolution"));
    }
    if px.len() as u64 != u64::from(width) * u64::from(height) * o.color_model.channels() as u64 {
        return Err("JPEG encoding failed: pixel buffer doesn't match the image size".into());
    }
    let layout = match o.color_model {
        ColorModel::Rgb => ChannelLayout::Rgb,
        ColorModel::Cmyk => ChannelLayout::Cmyk,
        ColorModel::Gray => ChannelLayout::Gray,
    };
    let mut img = Image::from_u8(width, height, layout, px.to_vec()).map_err(|e| format!("JPEG encoding failed: {e}"))?;
    if let Some(ppi) = ppi.filter(|p| p.is_finite() && *p >= 1.0) {
        let d = ppi.round().clamp(1.0, f64::from(u16::MAX)) as f32;
        img.meta.dpi = Some((d, d));
    }
    if o.embed_icc {
        let icc = cms::icc_bytes(&o.color_model.profile()).map_err(|e| format!("JPEG colour profile: {e}"))?;
        img.icc = Some(icc.to_vec());
    }
    let (jpeg_format, jpeg_progressive_scans) = match o.method {
        Method::Baseline => (JpegFormat::Baseline, None),
        Method::Optimized => (JpegFormat::Optimized, None),
        Method::Progressive => (JpegFormat::Progressive, Some(o.scans.clamp(*SCANS.start(), *SCANS.end()))),
    };
    let opts = EncodeOptions {
        jpeg_quality: quality.clamp(1, 100),
        jpeg_chroma_subsampling: false,
        jpeg_format,
        jpeg_progressive_scans,
        embed_icc: o.embed_icc,
        embed_metadata: true,
        ..Default::default()
    };
    craft_image_codecs::encode(&img, Format::Jpeg, &opts).map_err(|e| format!("JPEG encoding failed: {e}"))
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
