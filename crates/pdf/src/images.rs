//! Images in the PDF: the document's images as the Compression settings say (resampled above a
//! resolution, compressed with ZIP or JPEG) and converted as the Output settings say
//! ([`Pixels`]), and pixels the writer makes itself (freeform gradients).
//!
//! Every image is classified as the settings are split: colour, greyscale (a grey colour type) or
//! monochrome (greyscale that is only black and white). An image already as asked (not resampled,
//! a JPEG kept as JPEG or another kept lossless) is embedded as it is; the others are decoded,
//! resampled and encoded again. JPEG needs opaque pixels: images with transparency stay lossless.
//! A gray or RGB image that carries a profile the Output settings convert from ([`ToSrgb`]) is
//! always decoded: its pixels are converted to sRGB and written without the profile, at 16 bits a
//! sample when it has them and is otherwise kept as it is; a JPEG kept as JPEG whose samples the
//! conversion leaves unchanged (an sRGB profile) keeps its JPEG data. A black-and-white image whose
//! converted levels are grey is compressed as greyscale. A colour image the Output settings also
//! convert to their destination ([`Pixels`]) goes from its profile to sRGB, then to the
//! destination, and so does one converted from its profile to an RGB destination whose untagged
//! images keep their numbers (Preserve Numbers); a grey one keeps its profile there, as grey
//! pixels are not converted.
//!
//! CMYK images ([`ImageBlob::cmyk`]) stay CMYK: a CMYK JPEG already as asked is embedded as it is
//! when its numbers are kept; the others' ink amounts are resampled, converted when the Output
//! settings convert CMYK colours ([`CmykPixels`]) and compressed again. CMYK JPEGs hold Adobe's
//! inverted samples, which the PDF writer's Decode array undoes. A CMYK image whose numbers are
//! kept carries the CMYK profile it is given ([`cmyk_icc`]), whether it is embedded as it is or
//! written again; without one it is untagged, a JPEG's own `ICC_PROFILE` segments included.
//! With Don't Include Profiles set ([`SourceProfiles::Remove`], [`CmykTag::Removed`]), a JPEG
//! embedded as it is loses its `ICC_PROFILE` segments, its other segments and compressed samples
//! unchanged.

use std::io::Cursor;
use std::sync::Arc;

use craft::pdf::write::backend as krilla;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageFormat, ImageReader};
use krilla::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};

use vectorcraft_color::cms::{IccProfile, Intent};
use vectorcraft_doc::ImageBlob;
use vectorcraft_doc::cmyk::{Inks, jpeg_components};
use vectorcraft_doc::resample::{Resample, premultiply, resized, unpremultiply};

use crate::output::{CmykPixels, Pixels};
use crate::{CompressionSettings, Downsample, ImageCodec, JpegQuality, MonoCodec};

/// Raw pixels: colour samples (RGB, grey or CMYK) and an optional alpha channel, of 8 bits a
/// sample or of 16 (big-endian), with the ICC profile the colour samples are in, if any.
#[derive(Clone, Hash)]
struct Raw {
    color: Arc<Vec<u8>>,
    alpha: Option<Arc<Vec<u8>>>,
    width: u32,
    height: u32,
    space: ImageColorspace,
    bits: BitsPerComponent,
    icc: Option<Arc<Vec<u8>>>,
}

impl CustomImage for Raw {
    fn color_channel(&self) -> &[u8] {
        &self.color
    }

    fn alpha_channel(&self) -> Option<&[u8]> {
        self.alpha.as_deref().map(Vec::as_slice)
    }

    fn bits_per_component(&self) -> BitsPerComponent {
        self.bits
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn icc_profile(&self) -> Option<&[u8]> {
        self.icc.as_deref().map(Vec::as_slice)
    }

    fn color_space(&self) -> ImageColorspace {
        self.space
    }
}

/// `icc` when it is a well-formed profile ([`valid_icc`]) of the colour space `space` names: `GRAY`
/// for grey samples, `RGB ` for RGB ones. `None` for other profiles and spaces.
fn icc_for(icc: Option<Vec<u8>>, space: ImageColorspace) -> Option<Arc<Vec<u8>>> {
    let want = match space {
        ImageColorspace::Luma => *b"GRAY",
        ImageColorspace::Rgb => *b"RGB ",
        ImageColorspace::Cmyk | ImageColorspace::DeviceN => return None,
    };
    icc.filter(|icc| valid_icc(icc, want)).map(Arc::new)
}

/// The ICC profile image `bytes` carries: a TIFF's tag 34675 (read by the TIFF decoder itself, up
/// to 16 MiB), another format's as its decoder reads it. `None` without one.
pub(crate) fn source_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    if reader.format() == Some(ImageFormat::Tiff) {
        return vectorcraft_doc::cmyk::tiff_icc_profile(bytes);
    }
    let mut decoder = reader.into_decoder().ok()?;
    decoder.icc_profile().ok().flatten().filter(|icc| !icc.is_empty())
}

/// A `width` × `height` image of straight (not premultiplied) RGBA pixels, written as RGB (as
/// CMYK when `pixels` converts to CMYK), or as grey when every pixel is grey, with an alpha
/// channel only when some pixel isn't opaque. With an RGB `icc` profile, pixels that `pixels`
/// doesn't convert stay RGB and carry it; a grey one tags grey pixels. `interpolate` asks viewers
/// to smooth it when enlarged (PDF/A forbids it). `None` when `rgba` doesn't hold that many pixels.
fn from_rgba(rgba: &[u8], width: u32, height: u32, interpolate: bool, pixels: Option<&Pixels>, icc: Option<Vec<u8>>) -> Option<Image> {
    let px = rgba.as_chunks::<4>().0;
    if px.len() as u64 != width as u64 * height as u64 || px.is_empty() {
        return None;
    }
    let rgb_icc = pixels.is_none() && icc.as_deref().is_some_and(|icc| valid_icc(icc, *b"RGB "));
    let (color, space) = if neutral(rgba) && !rgb_icc {
        (px.iter().map(|p| p[0]).collect(), ImageColorspace::Luma)
    } else if let Some(cmyk) = pixels.and_then(|p| p.cmyk(rgba)) {
        (cmyk, ImageColorspace::Cmyk)
    } else {
        (px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(), ImageColorspace::Rgb)
    };
    let icc = icc_for(icc.filter(|_| pixels.is_none()), space);
    let alpha = px.iter().any(|p| p[3] < 255).then(|| Arc::new(px.iter().map(|p| p[3]).collect()));
    Image::from_custom(Raw { color: Arc::new(color), alpha, width, height, space, bits: BitsPerComponent::Eight, icc }, interpolate).ok()
}

/// Decoded image `img` written losslessly as it is: grey or RGB samples (other kinds as RGB), with
/// an alpha channel only when some pixel isn't opaque, at 16 bits a sample when it has more than 8
/// and `sixteen_bit` allows it (PDF 1.5 and later), else at 8; tagged with ICC profile `icc` when
/// that is a profile of the samples' colour space (`GRAY` or `RGB `). `None` when it has no pixels.
pub(crate) fn from_decoded(img: DynamicImage, icc: Option<Vec<u8>>, interpolate: bool, sixteen_bit: bool) -> Option<Image> {
    let (width, height) = (img.width(), img.height());
    if width == 0 || height == 0 {
        return None;
    }
    let kind = img.color();
    let space = if kind.has_color() { ImageColorspace::Rgb } else { ImageColorspace::Luma };
    let n = if kind.has_color() { 3 } else { 1 };
    let deep = sixteen_bit && kind.bits_per_pixel() > 8 * u16::from(kind.channel_count());
    let (color, alpha, bits) = if deep {
        let s = if kind.has_color() { img.into_rgba16().into_raw() } else { img.into_luma_alpha16().into_raw() };
        let px = s.chunks_exact(n + 1);
        let color = px.clone().flat_map(|p| p[..n].iter().flat_map(|v| v.to_be_bytes())).collect();
        let alpha = (kind.has_alpha() && px.clone().any(|p| p[n] < u16::MAX)).then(|| px.flat_map(|p| p[n].to_be_bytes()).collect());
        (color, alpha, BitsPerComponent::Sixteen)
    } else {
        let s = if kind.has_color() { img.into_rgba8().into_raw() } else { img.into_luma_alpha8().into_raw() };
        let px = s.chunks_exact(n + 1);
        let color = px.clone().flat_map(|p| p[..n].iter().copied()).collect();
        let alpha = (kind.has_alpha() && px.clone().any(|p| p[n] < u8::MAX)).then(|| px.map(|p| p[n]).collect());
        (color, alpha, BitsPerComponent::Eight)
    };
    let raw = Raw { color: Arc::new(color), alpha: alpha.map(Arc::new), width, height, space, bits, icc: icc_for(icc, space) };
    Image::from_custom(raw, interpolate).ok()
}

/// Image `bytes` decoded within [`limits`], with the ICC profile it carries ([`source_icc`]) when
/// `profiles` keeps it ([`SourceProfiles::Keep`]), as [`from_decoded`] writes it. `None` when it
/// can't be decoded.
pub(crate) fn decoded_image(bytes: &[u8], interpolate: bool, sixteen_bit: bool, profiles: SourceProfiles) -> Option<Image> {
    from_decoded(decoded(bytes)?, (profiles == SourceProfiles::Keep).then(|| source_icc(bytes)).flatten(), interpolate, sixteen_bit)
}

/// Are all of the RGBA pixels `rgba` grey?
fn neutral(rgba: &[u8]) -> bool {
    rgba.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2])
}

/// Colour `samples` (grey, RGB, or CMYK ink amounts, as `space` says) with an optional `alpha`
/// channel, written losslessly, tagged with ICC profile `icc` when given.
fn lossless(
    samples: Vec<u8>,
    alpha: Option<Vec<u8>>,
    space: ImageColorspace,
    width: u32,
    height: u32,
    interpolate: bool,
    icc: Option<&[u8]>,
) -> Option<Image> {
    let icc = icc.map(|icc| Arc::new(icc.to_vec()));
    let raw = Raw { color: Arc::new(samples), alpha: alpha.map(Arc::new), width, height, space, bits: BitsPerComponent::Eight, icc };
    Image::from_custom(raw, interpolate).ok()
}

/// Most pixels an image may have to be embedded: twice what the app reads (64 million), so every image
/// it holds is still written.
pub(crate) const MAX_EMBED_PIXELS: u64 = 1 << 27;

/// The most bytes an ICC profile carried by an image is written or read with (16 MiB).
pub(crate) const MAX_ICC: usize = 16 << 20;

/// The CMYK ICC profile image `blob` carries ([`ImageBlob::cmyk_icc_profile`]), when it is one a
/// CMYK image can be tagged with ([`valid_icc`]). `None` for other images and other profiles.
pub(crate) fn cmyk_icc(blob: &ImageBlob) -> Option<Vec<u8>> {
    blob.cmyk_icc_profile().filter(|icc| valid_icc(icc, *b"CMYK"))
}

/// Is `icc` a profile an image of colour space `space` (`GRAY`, `RGB ` or `CMYK`) can be tagged
/// with: at most [`MAX_ICC`] bytes, the size its header declares, the `acsp` signature, an input,
/// display, output or colour space profile of `space` colours, and a tag table whose tags lie
/// within it?
pub(crate) fn valid_icc(icc: &[u8], space: [u8; 4]) -> bool {
    let be = |at: usize| icc.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes);
    let fits = |at: u32, len: u32| u64::from(at) + u64::from(len) <= icc.len() as u64;
    if icc.len() > MAX_ICC || be(0).and_then(|n| usize::try_from(n).ok()) != Some(icc.len()) {
        return false;
    }
    let class = icc.get(12..16);
    if icc.get(36..40) != Some(b"acsp".as_slice())
        || icc.get(16..20) != Some(space.as_slice())
        || ![b"scnr", b"mntr", b"prtr", b"spac"].iter().any(|c| class == Some(c.as_slice()))
    {
        return false;
    }
    let Some(count) = be(128) else { return false };
    if !fits(132, count.saturating_mul(12)) {
        return false;
    }
    (0..count as usize).all(|i| {
        let at = 132 + i * 12;
        matches!((be(at + 4), be(at + 8)), (Some(offset), Some(len)) if fits(offset, len))
    })
}

/// [`from_rgba`] of pixels converted by `pixels` (when given).
pub(crate) fn from_rgba_in(mut rgba: Vec<u8>, width: u32, height: u32, interpolate: bool, pixels: Option<&Pixels>) -> Option<Image> {
    if let Some(p) = pixels {
        p.rgb_in_place(&mut rgba);
    }
    from_rgba(&rgba, width, height, interpolate, pixels, None)
}

/// A `width` × `height` image of CMYK ink amounts `inks` (four bytes a pixel), converted by
/// `convert` (when given), with straight `alpha` (one byte a pixel) carried only when some pixel
/// isn't opaque. `None` when the buffers don't hold that many pixels.
pub(crate) fn from_inks(inks: Vec<u8>, alpha: Vec<u8>, width: u32, height: u32, interpolate: bool, convert: Option<&CmykPixels>) -> Option<Image> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || inks.len() as u64 != pixels * 4 || alpha.len() as u64 != pixels {
        return None;
    }
    let alpha = alpha.iter().any(|a| *a < 255).then_some(alpha);
    let (samples, space) = match convert {
        Some(p) => p.convert(&inks),
        None => (inks, ImageColorspace::Cmyk),
    };
    lossless(samples, alpha, space, width, height, interpolate, None)
}

/// The kinds of images the Compression section has settings for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Color,
    Gray,
    Mono,
}

/// How an image's pixels are compressed in the file.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Codec {
    /// JPEG images stay JPEG (at this quality when they are resampled); the others are lossless.
    Auto(u8),
    Zip,
    Jpeg(u8),
}

/// The settings for one kind of image.
struct Rule {
    downsample: Downsample,
    /// Target resolution, and the resolution above which images are resampled (ppi).
    ppi: f64,
    above: f64,
    codec: Codec,
    /// Why the codec differs from the one asked for (that one isn't available).
    instead: Option<&'static str>,
}

const UNCOMPRESSED: &str = "uncompressed images are not written: images are compressed with ZIP";
const CMYK_KEPT: &str = "a CMYK image couldn't be decoded: it is written as it is, not resampled or converted";
const CMYK_RGB: &str = "a CMYK image couldn't be decoded in CMYK: it is written in RGB";

/// JPEG quality (1–100) of a quality setting.
fn quality(q: JpegQuality) -> u8 {
    match q {
        JpegQuality::Minimum => 10,
        JpegQuality::Low => 30,
        JpegQuality::Medium => 55,
        JpegQuality::High => 80,
        JpegQuality::Maximum => 95,
    }
}

fn rule(c: &CompressionSettings, kind: Kind) -> Rule {
    if kind == Kind::Mono {
        let m = &c.mono;
        let instead = match m.compression {
            MonoCodec::Zip => None,
            MonoCodec::None => Some(UNCOMPRESSED),
            MonoCodec::CcittG3 | MonoCodec::CcittG4 => Some("CCITT compression is not supported: monochrome images are compressed with ZIP"),
            MonoCodec::RunLength => Some("Run Length compression is not supported: monochrome images are compressed with ZIP"),
        };
        return Rule { downsample: m.downsample, ppi: m.ppi, above: m.above_ppi, codec: Codec::Zip, instead };
    }
    let i = if kind == Kind::Color { &c.color } else { &c.gray };
    let (codec, instead) = match i.compression {
        ImageCodec::Auto => (Codec::Auto(quality(i.quality)), None),
        ImageCodec::Zip => (Codec::Zip, None),
        ImageCodec::Jpeg => (Codec::Jpeg(quality(i.quality)), None),
        ImageCodec::None => (Codec::Zip, Some(UNCOMPRESSED)),
        ImageCodec::Jpeg2000 => (Codec::Zip, Some("JPEG 2000 compression is not supported: those images are compressed with ZIP")),
    };
    Rule { downsample: i.downsample, ppi: i.ppi, above: i.above_ppi, codec, instead }
}

/// What [`recode`] makes of an image.
pub(crate) struct Recoded {
    /// The image to write; `None` keeps the source image as it is.
    pub image: Option<Image>,
    /// Set when the image isn't compressed as asked.
    pub warning: Option<&'static str>,
    /// Set when the image's colors are converted to the destination without its ICC profile,
    /// which can't be read.
    pub note: Option<&'static str>,
    /// Set when the colors were to be converted but the image couldn't be decoded within the
    /// allowance or written with them: the source image, kept as it is, would be in its own colors.
    pub unconverted: bool,
}

/// How [`recode`] converts a gray or RGB image that carries an ICC profile to sRGB
/// ([`crate::output::ColorOut::image_srgb_intent`]): the destination when it is sRGB, else the
/// step before the image's [`Pixels`] conversion.
#[derive(Clone, Copy)]
pub(crate) struct ToSrgb<'a> {
    /// The rendering intent of the conversion.
    pub intent: Intent,
    /// 16-bit samples may be written (PDF 1.5 and later).
    pub sixteen_bit: bool,
    /// The colour output whose [`crate::output::ColorOut::tagged_pixels`] then convert a colour
    /// image converted from its profile, when other colour images are not converted (`recode`'s
    /// `convert`): to an RGB destination whose untagged images keep their numbers (Preserve
    /// Numbers). Its table is built when the first such image needs it.
    pub then: Option<&'a crate::output::ColorOut>,
    /// Gray images are converted too: with an sRGB destination
    /// ([`crate::output::ColorOut::converts_grey_images`]). In the others they keep their profile.
    pub grey: bool,
}

/// What happens to the ICC profile a gray or RGB image carries
/// ([`crate::output::ColorOut::source_profiles`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceProfiles {
    /// The image keeps a well-formed gray or RGB profile ([`valid_icc`]).
    Keep,
    /// The image is untagged; a JPEG embedded as it is keeps its data unchanged (PDF/X-1a).
    Untag,
    /// The image is untagged and a JPEG embedded as it is loses its `ICC_PROFILE` segments, its
    /// other segments and compressed samples unchanged (Don't Include Profiles).
    Remove,
}

/// The CMYK profile [`recode_cmyk`] tags a CMYK image that keeps its numbers with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CmykTag<'a> {
    /// This profile ([`cmyk_icc`]).
    Profile(&'a [u8]),
    /// None; a JPEG embedded as it is keeps its data unchanged.
    Untagged,
    /// None, and a JPEG embedded as it is loses its `ICC_PROFILE` segments, its other segments
    /// and compressed samples unchanged (Don't Include Profiles).
    Removed,
}

impl<'a> CmykTag<'a> {
    /// The profile the image is tagged with.
    fn icc(self) -> Option<&'a [u8]> {
        match self {
            Self::Profile(icc) => Some(icc),
            Self::Untagged | Self::Removed => None,
        }
    }
}

/// JPEG `bytes` as they are embedded: without their `ICC_PROFILE` segments with `remove`
/// ([`vectorcraft_doc::cmyk::jpeg_without_icc`]). `None` when those can't be taken out.
pub(crate) fn jpeg_data(bytes: &[u8], remove: bool) -> Option<Vec<u8>> {
    if remove { vectorcraft_doc::cmyk::jpeg_without_icc(bytes) } else { Some(bytes.to_vec()) }
}

/// The ICC profile image `bytes` carries ([`source_icc`]), when it is a well-formed profile of
/// colour space `space` (`GRAY` or `RGB `, [`valid_icc`]): `None` without one, `Some(None)` when
/// the CMS can't read it.
fn source_profile(bytes: &[u8], space: [u8; 4]) -> Option<Option<IccProfile>> {
    let icc = source_icc(bytes).filter(|icc| valid_icc(icc, space))?;
    Some(IccProfile::from_bytes(None, &icc).ok())
}

const UNCONVERTED: &str = "an image's ICC profile couldn't be read: the image keeps it and is not converted to sRGB";
const CONVERTED_AS_SRGB: &str = "an image's ICC profile couldn't be read: its colors are converted to the destination as sRGB";

const UNDECODED: &str = "an image couldn't be decoded and is not converted to sRGB";

/// `img` converted from ICC profile `p` to sRGB with `intent`: at 16 bits a sample when it has
/// more than 8, else at 8, with its alpha; and whether any sample changed. A grey image stays grey, each level as the sRGB grey it converts to. `None` when the
/// pixels can't be converted.
fn into_srgb(img: DynamicImage, p: &IccProfile, intent: Intent) -> Option<(DynamicImage, bool)> {
    let kind = img.color();
    let deep = kind.bits_per_pixel() > 8 * u16::from(kind.channel_count());
    let to8 = |v: f32| v.round() as u8;
    let to16 = |v: f32| v.round() as u16;
    Some(match (kind.has_color(), deep) {
        (true, true) => {
            let mut s = img.into_rgba16();
            let changed = convert_samples(&mut s, 3, p, intent, 65_535.0, to16)?;
            (DynamicImage::ImageRgba16(s), changed)
        }
        (true, false) => {
            let mut s = img.into_rgba8();
            let changed = convert_samples(&mut s, 3, p, intent, 255.0, to8)?;
            (DynamicImage::ImageRgba8(s), changed)
        }
        (false, true) => {
            let mut s = img.into_luma_alpha16();
            let changed = convert_samples(&mut s, 1, p, intent, 65_535.0, to16)?;
            (DynamicImage::ImageLumaA16(s), changed)
        }
        (false, false) => {
            let mut s = img.into_luma_alpha8();
            let changed = convert_samples(&mut s, 1, p, intent, 255.0, to8)?;
            (DynamicImage::ImageLumaA8(s), changed)
        }
    })
}

/// Convert the `n` colour samples (1: grey, 3: RGB) of each pixel of `samples` (followed by an
/// alpha sample each, which is left as it is) from ICC profile `p` to sRGB with `intent`, a run of
/// pixels at a time; `max` is a sample's largest value, `back` turns a value of 0 to `max` into a
/// sample. Grey pixels take the mean of the sRGB values. Whether any sample changed; `None` when
/// the transform fails.
fn convert_samples<T: Copy + PartialEq + Into<f32>>(
    samples: &mut [T],
    n: usize,
    p: &IccProfile,
    intent: Intent,
    max: f32,
    back: impl Fn(f32) -> T,
) -> Option<bool> {
    const RUN: usize = 4096;
    let mut src = Vec::with_capacity(RUN * n);
    let mut dst = vec![0.0f32; RUN * 3];
    let mut changed = false;
    for run in samples.chunks_mut(RUN * (n + 1)) {
        src.clear();
        src.extend(run.chunks_exact(n + 1).flat_map(|px| px.iter().take(n).map(|v| (*v).into() / max)));
        let out = dst.get_mut(..src.len() / n * 3)?;
        p.to_srgb_pixels(&src, out, intent)?;
        for (px, rgb) in run.chunks_exact_mut(n + 1).zip(out.as_chunks::<3>().0) {
            let grey = [rgb.iter().sum::<f32>() / 3.0];
            let values: &[f32] = if n == 1 { &grey } else { rgb };
            for (v, c) in px.iter_mut().zip(values) {
                let new = back(c * max);
                changed |= new != *v;
                *v = new;
            }
        }
    }
    Some(changed)
}

/// Is `img` (a grey image) only black and white?
fn bilevel(img: &DynamicImage) -> bool {
    img.to_luma_alpha8().pixels().all(|p| p[0] == 0 || p[0] == 255)
}

/// The size `r` gives an image of `w` × `h` pixels placed `size` points wide and high: a side
/// whose resolution is above the threshold goes down to the target resolution.
fn sized(r: &Rule, (w, h): (u32, u32), size: (f64, f64)) -> (u32, u32) {
    let ppi = |px: u32, pt: f64| px as f64 * 72.0 / pt.abs();
    let (px, py) = (ppi(w, size.0), ppi(h, size.1));
    let resample = r.downsample != Downsample::None && [px, py].iter().any(|p| p.is_finite() && *p > r.above);
    let side = |n: u32, p: f64| if resample && p.is_finite() && p > r.ppi { ((n as f64 * r.ppi / p).round() as u32).clamp(1, n) } else { n };
    (side(w, px), side(h, py))
}

/// Is an image of `dims` pixels placed `size` points already as `r` asks: not resampled, and
/// lossless or (`jpeg`) a JPEG kept as JPEG?
fn kept(r: &Rule, dims: (u32, u32), size: (f64, f64), jpeg: bool) -> bool {
    sized(r, dims, size) == dims
        && match r.codec {
            Codec::Auto(_) => true,
            Codec::Zip => !jpeg,
            Codec::Jpeg(_) => jpeg,
        }
}

/// The JPEG quality `r` compresses an image with (`jpeg`: it is a JPEG); `None`: lossless.
fn jpeg_quality(r: &Rule, jpeg: bool) -> Option<u8> {
    match r.codec {
        Codec::Jpeg(q) => Some(q),
        Codec::Auto(q) if jpeg => Some(q),
        _ => None,
    }
}

/// How `r` resamples an image.
fn resampling(r: &Rule) -> Resample {
    match r.downsample {
        Downsample::Average => Resample::Average,
        Downsample::Subsample => Resample::Filter(FilterType::Nearest),
        Downsample::Bicubic | Downsample::None => Resample::Filter(FilterType::CatmullRom),
    }
}

/// `img` resampled to `w` × `h` pixels as `r` says, the colors of an image with alpha weighted by
/// it ([`resized`]).
fn resample(img: DynamicImage, r: &Rule, (w, h): (u32, u32)) -> DynamicImage {
    resized(img, w, h, resampling(r))
}

/// The four channels of `img` resampled to `w` × `h` pixels as `r` says, each on its own.
fn resample_channels(img: DynamicImage, r: &Rule, (w, h): (u32, u32)) -> DynamicImage {
    if (w, h) == (img.width(), img.height()) {
        return img;
    }
    match resampling(r) {
        Resample::Average => img.thumbnail_exact(w, h),
        Resample::Filter(f) => img.resize_exact(w, h, f),
    }
}

/// Are samples of type `t` grey (with or without alpha)? [`recode`] keeps such pixels grey.
fn grey_type(t: ExtendedColorType) -> bool {
    use ExtendedColorType as T;
    matches!(t, T::L1 | T::La1 | T::L2 | T::La2 | T::L4 | T::La4 | T::L8 | T::La8 | T::L16 | T::La16)
}

/// Does the image in `bytes` hold grey samples, which [`recode`] keeps grey whatever the Output
/// settings? Reads only its header; `false` for one that can't be read.
pub(crate) fn is_grey(bytes: &[u8]) -> bool {
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_decoder().ok())
        .is_some_and(|d| grey_type(d.original_color_type()))
}

/// Encoded image `bytes` placed `size` points wide and high, as the Compression settings `c` ask:
/// resampled when its resolution is above the threshold, compressed with ZIP or JPEG, its colours
/// converted by `convert` (colour images). Pixels that keep their colours keep the ICC profile
/// they carry when `profiles` keeps it ([`SourceProfiles::Keep`]); with `to_srgb`, pixels in a
/// gray or RGB profile other than sRGB are converted from it to sRGB instead ([`ToSrgb`]), and
/// colour ones then by `convert`, or without it by the conversion `to_srgb` gives
/// ([`ToSrgb::then`]); grey ones keep their profile unless the destination is sRGB
/// ([`ToSrgb::grey`]). `interpolate` lets viewers smooth it. An image that can't be decoded is
/// kept as it is, with a warning when its pixels were to be converted to sRGB. When `convert`
/// was to convert its colors, it is [`Recoded::unconverted`] if its header can't be read (it may
/// be in color) or its pixels are beyond the decode allowance; a color image whose pixel data
/// can't be read is not.
pub(crate) fn recode(
    bytes: &[u8],
    size: (f64, f64),
    c: &CompressionSettings,
    interpolate: bool,
    convert: Option<&Pixels>,
    profiles: SourceProfiles,
    to_srgb: Option<ToSrgb>,
) -> Recoded {
    let keep = Recoded { image: None, warning: None, note: None, unconverted: convert.is_some() };
    let Ok(reader) = ImageReader::new(Cursor::new(bytes)).with_guessed_format() else { return keep };
    let jpeg = reader.format() == Some(ImageFormat::Jpeg);
    let Ok(mut decoder) = reader.into_decoder() else { return keep };
    let dims = decoder.dimensions();
    let grey = grey_type(decoder.original_color_type());
    // Pixels in a profile they are converted from are decoded, whatever the Compression settings.
    // Grey pixels keep their profile unless the destination is sRGB ([`ToSrgb::grey`]).
    let then = to_srgb.and_then(|t| t.then);
    let to_srgb = to_srgb.filter(|t| !grey || t.grey);
    // Converted to the destination as sRGB, as before, when the profile can't be read.
    let mut note = None;
    let source = match to_srgb.and_then(|t| Some((source_profile(bytes, if grey { *b"GRAY" } else { *b"RGB " })?, t))) {
        Some((Some(p), t)) => Some((p, t)),
        Some((None, _)) if convert.is_some() => {
            note = Some(CONVERTED_AS_SRGB);
            None
        }
        Some((None, _)) => return Recoded { image: None, warning: Some(UNCONVERTED), note: None, unconverted: false },
        None => None,
    };
    // Pixels that can't be decoded are kept as they are, with a warning when they were to be
    // converted.
    let keep = if source.is_some() { Recoded { warning: Some(UNDECODED), ..keep } } else { keep };
    // What the header declares is decoded within the image allowance, not whatever it asks for.
    // Grey pixels aren't converted.
    if decoder.set_limits(limits()).is_err() {
        return Recoded { unconverted: convert.is_some() && !grey, ..keep };
    }
    let (mut kind, pixels) = if grey {
        // Only the pixels tell monochrome images from greyscale ones: decoded unless both keep it.
        let (g, m) = (rule(c, Kind::Gray), rule(c, Kind::Mono));
        if source.is_none() && kept(&g, dims, size, jpeg) && kept(&m, dims, size, jpeg) && g.instead == m.instead {
            return Recoded { image: None, warning: g.instead, note: None, unconverted: false };
        }
        // Grey pixels aren't converted.
        let Ok(img) = DynamicImage::from_decoder(decoder) else { return Recoded { unconverted: false, ..keep } };
        (if bilevel(&img) { Kind::Mono } else { Kind::Gray }, Some(img))
    } else {
        (Kind::Color, None)
    };
    let mut rule = rule(c, kind);
    let convert = convert.filter(|_| kind == Kind::Color);
    if kept(&rule, dims, size, jpeg) && convert.is_none() && source.is_none() {
        return Recoded { image: None, warning: rule.instead, note: None, unconverted: false };
    }
    // Pixels that can't be read can't be written in their own colors either; ones beyond the
    // allowance can.
    let mut img = match pixels.map_or_else(|| decoded_within(bytes), Ok) {
        Ok(img) => img,
        Err(limited) => return Recoded { unconverted: limited && convert.is_some(), ..keep },
    };
    // Converted from its profile: then by `convert`, else by the conversion `to_srgb` gives.
    let mut from_profile = false;
    if let Some((p, t)) = &source {
        let srgb = match into_srgb(img, p, t.intent) {
            Some(s) => Some(s),
            None if convert.is_some() => {
                // Converted to the destination as sRGB: decoded again (a colour image), as the
                // conversion took the pixels.
                note = Some(CONVERTED_AS_SRGB);
                None
            }
            None => return Recoded { image: None, warning: Some(UNCONVERTED), note: None, unconverted: false },
        };
        img = match srgb {
            Some((srgb, changed)) => {
                // The converted levels of a black-and-white image may be grey.
                if kind == Kind::Mono && !bilevel(&srgb) {
                    (kind, rule) = (Kind::Gray, self::rule(c, Kind::Gray));
                }
                from_profile = true;
                // Written as it is only when nothing converts it further.
                let as_is = kept(&rule, dims, size, jpeg) && convert.is_none() && then.is_none();
                // A JPEG kept as JPEG whose samples the conversion leaves unchanged (an sRGB
                // profile): its JPEG data, without the profile (and, with `Remove`, without its
                // segments: encoded again when those can't be taken out).
                if !changed
                    && as_is
                    && jpeg
                    && let Some(data) = jpeg_data(bytes, profiles == SourceProfiles::Remove)
                {
                    let image = Image::from_jpeg_with_icc(data.into(), None, interpolate).ok();
                    return Recoded { image, warning: rule.instead, note: None, unconverted: false };
                }
                // Not resampled and lossless: written at its own depth.
                if as_is && jpeg_quality(&rule, jpeg).is_none() {
                    let image = from_decoded(srgb, None, interpolate, t.sixteen_bit);
                    return Recoded { image, warning: rule.instead, note: None, unconverted: false };
                }
                srgb
            }
            None => match decoded_within(bytes) {
                Ok(img) => img,
                Err(limited) => return Recoded { unconverted: limited && convert.is_some(), ..keep },
            },
        };
    }
    let convert = if from_profile { convert.or_else(|| then.and_then(crate::output::ColorOut::tagged_pixels)) } else { convert };
    // Pixels that keep their colours keep the profile they are in; converted ones are in the
    // destination's.
    let want = if kind == Kind::Color { *b"RGB " } else { *b"GRAY" };
    let keeps = profiles == SourceProfiles::Keep && convert.is_none() && source.is_none();
    let icc = keeps.then(|| source_icc(bytes)).flatten().filter(|icc| valid_icc(icc, want));
    let rgb_icc = icc.is_some() && kind == Kind::Color;
    let mut rgba = resample(img, &rule, sized(&rule, dims, size)).into_rgba8();
    if kind == Kind::Mono {
        // Resampling greys the edges: back to black and white.
        for p in rgba.pixels_mut() {
            let v = if p[0] >= 128 { 255 } else { 0 };
            (p[0], p[1], p[2]) = (v, v, v);
        }
    }
    if let Some(p) = convert {
        p.rgb_in_place(&mut rgba);
    }
    let (width, height) = rgba.dimensions();
    let opaque = rgba.pixels().all(|p| p[3] == 255);
    let jpeg = jpeg_quality(&rule, jpeg).filter(|_| opaque).and_then(|q| {
        // Grey pixels stay grey, unless they keep an RGB profile; colour ones go to CMYK when the
        // output converts to CMYK.
        let grey = kind != Kind::Color || (neutral(&rgba) && !rgb_icc);
        let (samples, space) = match convert.filter(|_| !grey).and_then(|p| p.cmyk(&rgba)) {
            Some(cmyk) => (cmyk, ImageColorspace::Cmyk),
            None if grey => (rgba.pixels().map(|p| p[0]).collect(), ImageColorspace::Luma),
            None => (rgba.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect(), ImageColorspace::Rgb),
        };
        let icc = icc_for(icc.clone(), space);
        to_jpeg(&samples, space, width, height, q, interpolate, icc.as_deref().map(Vec::as_slice))
    });
    let image = jpeg.or_else(|| from_rgba(rgba.as_raw(), width, height, interpolate, convert, icc));
    let unconverted = image.is_none() && convert.is_some();
    Recoded { image, warning: rule.instead, note, unconverted }
}

/// Whether coverage `cover` (one byte a pixel) only covers or doesn't, as a 1-bit image mask's
/// does: what the Monochrome settings resample, and kept so.
pub(crate) fn bilevel_coverage(cover: &[u8]) -> bool {
    cover.iter().all(|v| *v == 0 || *v == 255)
}

/// A colorized image's coverage (one sample a pixel, `w` × `h`, see
/// [`ImageBlob::coverage`]) placed `size` points wide and high, resampled as the Compression
/// settings `c` ask for monochrome images (when every sample is 0 or 255) or grayscale ones.
pub(crate) fn coverage_sized(w: u32, h: u32, cover: Vec<u8>, size: (f64, f64), c: &CompressionSettings) -> Option<(u32, u32, Vec<u8>)> {
    let mono = bilevel_coverage(&cover);
    let rule = rule(c, if mono { Kind::Mono } else { Kind::Gray });
    let to = sized(&rule, (w, h), size);
    if to == (w, h) {
        return Some((w, h, cover));
    }
    let img = DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, cover)?);
    let mut out = resample(img, &rule, to).into_luma8();
    if mono {
        // Resampling greys the edges: back to covered and not.
        for p in out.pixels_mut() {
            p[0] = if p[0] >= 128 { 255 } else { 0 };
        }
    }
    let (w, h) = out.dimensions();
    Some((w, h, out.into_raw()))
}

/// [`recode`] of CMYK image `blob` ([`ImageBlob::cmyk`]): a JPEG already as asked is kept as it
/// is when its numbers are (`convert` is `None`); otherwise its ink amounts are resampled,
/// converted by `convert` and compressed again. With `alpha` (one sample a pixel, see
/// [`mask_alpha`]) the image is lossless and carries it as its soft mask; `None` then when its
/// inks can't be read. When its numbers are kept, the image is tagged as `tag` says, and is
/// untagged without a profile; converted pixels are never tagged with it.
/// With `convert`, an image whose inks can't be read or written is [`Recoded::unconverted`].
pub(crate) fn recode_cmyk(
    blob: &ImageBlob,
    size: (f64, f64),
    c: &CompressionSettings,
    interpolate: bool,
    convert: Option<&CmykPixels>,
    alpha: Option<Vec<u8>>,
    tag: CmykTag<'_>,
) -> Recoded {
    let rule = rule(c, Kind::Color);
    let jpeg = jpeg_components(&blob.bytes).is_some();
    let icc = tag.icc().filter(|_| convert.is_none());
    if jpeg && convert.is_none() && alpha.is_none() {
        let dims = ImageReader::new(Cursor::new(blob.bytes.as_slice())).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok());
        // The JPEG data as it is (without its `ICC_PROFILE` segments with `CmykTag::Removed`: its
        // inks are encoded again when those can't be taken out), tagged with `icc` rather than
        // with its own segments; one larger than [`MAX_EMBED_PIXELS`] is not written.
        if let Some((w, h)) = dims.filter(|d| kept(&rule, *d, size, true))
            && let Some(data) = jpeg_data(&blob.bytes, tag == CmykTag::Removed)
        {
            let image = (u64::from(w) * u64::from(h) <= MAX_EMBED_PIXELS)
                .then(|| Image::from_jpeg_with_icc(data.into(), icc.map(|i| i.to_vec().into()), interpolate).ok())
                .flatten();
            return Recoded { image, warning: rule.instead, note: None, unconverted: false };
        }
    }
    // Inks that can't be read can't be converted: the image is not to be kept as it is.
    let unconverted = Recoded { image: None, warning: None, note: None, unconverted: true };
    let Some(inks) = blob.cmyk() else {
        if convert.is_some() {
            return unconverted;
        }
        return Recoded { image: None, warning: Some(if jpeg { CMYK_KEPT } else { CMYK_RGB }), note: None, unconverted: false };
    };
    let (width, height) = sized(&rule, (inks.width, inks.height), size);
    let (inks, alpha) = match resampled_inks(inks, alpha, &rule, (width, height)) {
        Ok(r) => r,
        Err(_) if convert.is_some() => return unconverted,
        Err(warning) => return Recoded { image: None, warning, note: None, unconverted: false },
    };
    let (samples, space) = match convert {
        Some(p) => p.convert(&inks),
        None => (inks, ImageColorspace::Cmyk),
    };
    // JPEG needs opaque pixels.
    let image = jpeg_quality(&rule, jpeg).filter(|_| alpha.is_none()).and_then(|q| to_jpeg(&samples, space, width, height, q, interpolate, icc));
    let image = image.or_else(|| lossless(samples, alpha, space, width, height, interpolate, icc));
    let unconverted = image.is_none() && convert.is_some();
    Recoded { image, warning: rule.instead, note: None, unconverted }
}

/// Ink amounts (four bytes a pixel) and their alpha (one byte a pixel), if any.
type InksAndAlpha = (Vec<u8>, Option<Vec<u8>>);

/// Ink amounts `inks` and their `alpha` (one sample a pixel) resampled to `to` as `r` says. The
/// inks under clear pixels don't count where neighbors blend: they are multiplied by the alpha
/// before resampling and divided by the resampled alpha after. `Err` with the warning when they
/// can't be read.
fn resampled_inks(inks: Inks, alpha: Option<Vec<u8>>, r: &Rule, to: (u32, u32)) -> Result<InksAndAlpha, Option<&'static str>> {
    let dims = (inks.width, inks.height);
    let weighted = to != dims && resampling(r) != Resample::Filter(FilterType::Nearest);
    let mut data = inks.data;
    let alpha = match alpha.map(|a| image::GrayImage::from_raw(dims.0, dims.1, a)) {
        Some(Some(a)) => {
            if weighted {
                premultiply(&mut data, a.as_raw());
            }
            Some(resample(DynamicImage::ImageLuma8(a), r, to).into_luma8().into_raw())
        }
        Some(None) => return Err(None),
        None => None,
    };
    // The four inks resample as the four channels of an RGBA image do, each on its own.
    let img = image::RgbaImage::from_raw(dims.0, dims.1, data).ok_or(Some(CMYK_RGB))?;
    let mut inks = resample_channels(DynamicImage::ImageRgba8(img), r, to).into_rgba8().into_raw();
    if let Some(a) = alpha.as_deref().filter(|_| weighted) {
        unpremultiply(&mut inks, a);
    }
    Ok((inks, alpha))
}

/// The most one image read here may allocate (bytes), as the image reader allows by default.
const MAX_DECODE: u64 = 512 << 20;

/// The decoder limits of an image read here: [`MAX_DECODE`] bytes.
fn limits() -> image::Limits {
    let mut l = image::Limits::default();
    l.max_alloc = Some(MAX_DECODE);
    l
}

/// `bytes` decoded within [`limits`]: an image whose header asks for more is not decoded.
fn decoded(bytes: &[u8]) -> Option<DynamicImage> {
    decoded_within(bytes).ok()
}

/// `bytes` decoded like [`decoded`]; on failure, whether the decoder stopped at [`limits`]
/// (`true`) rather than at data it can't read (`false`).
fn decoded_within(bytes: &[u8]) -> Result<DynamicImage, bool> {
    let mut r = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|_| false)?;
    r.limits(limits());
    r.decode().map_err(|e| matches!(e, image::ImageError::Limits(_)))
}

/// The alpha an opacity mask whose art is greyscale image `blob` alone gives the image of
/// `width` × `height` pixels it covers pixel for pixel: the mask's luminance, over black where the
/// mask image is transparent when it clips (`clip`), else over white. `None` when `blob` isn't
/// that many pixels or isn't grey (the luminance of colours is the soft mask's to work out).
pub(crate) fn mask_alpha(blob: &ImageBlob, width: u32, height: u32, clip: bool) -> Option<Vec<u8>> {
    if blob.is_cmyk() {
        return None;
    }
    // The size from the header first: a mask of another size, or too large to hold as RGBA, is
    // not decoded at all.
    let reader = ImageReader::new(Cursor::new(blob.bytes.as_slice())).with_guessed_format().ok()?;
    if reader.into_dimensions().ok()? != (width, height) || u64::from(width) * u64::from(height) * 4 > MAX_DECODE {
        return None;
    }
    let img = decoded(&blob.bytes)?.into_rgba8();
    if img.dimensions() != (width, height) || !neutral(img.as_raw()) {
        return None;
    }
    let back = if clip { 0 } else { 255 };
    Some(img.pixels().map(|p| ((u32::from(p[0]) * u32::from(p[3]) + back * (255 - u32::from(p[3])) + 127) / 255) as u8).collect())
}

/// The alpha of each pixel of image `blob` (255 for an image without alpha), when it is `width` ×
/// `height` pixels. `None` for a CMYK image, another size, or one too large to hold as RGBA or
/// that can't be decoded; as in [`mask_alpha`], the size is read from the header first.
pub(crate) fn image_alpha(blob: &ImageBlob, width: u32, height: u32) -> Option<Vec<u8>> {
    if blob.is_cmyk() {
        return None;
    }
    let reader = ImageReader::new(Cursor::new(blob.bytes.as_slice())).with_guessed_format().ok()?;
    if reader.into_dimensions().ok()? != (width, height) || u64::from(width) * u64::from(height) * 4 > MAX_DECODE {
        return None;
    }
    let img = decoded(&blob.bytes)?.into_rgba8();
    (img.dimensions() == (width, height)).then(|| img.pixels().map(|p| p[3]).collect())
}

/// Opaque colour `samples` (grey, RGB, or CMYK ink amounts, as `space` says) as a JPEG at
/// `quality`, tagged with ICC profile `icc` when given. CMYK JPEGs hold Adobe's inverted samples,
/// which the PDF writer's Decode array undoes.
fn to_jpeg(samples: &[u8], space: ImageColorspace, width: u32, height: u32, quality: u8, interpolate: bool, icc: Option<&[u8]>) -> Option<Image> {
    let mut out = Vec::new();
    match space {
        ImageColorspace::Cmyk => {
            let (w, h) = (u16::try_from(width).ok()?, u16::try_from(height).ok()?);
            jpeg_encoder::Encoder::new(&mut out, quality).encode(samples, w, h, jpeg_encoder::ColorType::Cmyk).ok()?;
        }
        ImageColorspace::Rgb | ImageColorspace::Luma => {
            let t = if space == ImageColorspace::Rgb { ExtendedColorType::Rgb8 } else { ExtendedColorType::L8 };
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
            if let Some(icc) = icc {
                image::ImageEncoder::set_icc_profile(&mut encoder, icc.to_vec()).ok()?;
            }
            encoder.encode(samples, width, height, t).ok()?;
        }
        ImageColorspace::DeviceN => return None,
    }
    Image::from_jpeg_with_icc(out.into(), icc.map(|i| i.to_vec().into()), interpolate).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The inks of a CMYK image with alpha, resampled, keep the amounts of the opaque pixels: the
    /// inks under the clear pixels beside them don't thin them out.
    #[test]
    fn resampled_inks_keep_the_amounts_beside_clear_pixels() {
        let data = [[40, 80, 120, 160], [0, 0, 0, 0]].repeat(8).concat();
        let alpha = [255, 0].repeat(8);
        for method in ["average", "bicubic"] {
            let c: CompressionSettings = serde_json::from_value(serde_json::json!({"color": {"downsample": method}})).unwrap();
            let inks = Inks::new(16, 1, data.clone()).unwrap();
            let (inks, alpha) = resampled_inks(inks, Some(alpha.clone()), &rule(&c, Kind::Color), (4, 1)).unwrap();
            let alpha = alpha.unwrap();
            let m = |c: u8, a: u8| (u16::from(c) * u16::from(a) / 255) as u8;
            for (p, &a) in inks.as_chunks::<4>().0.iter().zip(&alpha) {
                assert!(a > 0 && p.iter().zip([40, 80, 120, 160]).all(|(&c, want)| m(c, a).abs_diff(m(want, a)) <= 1), "{method}: {p:?} at {a}");
            }
        }
    }
}
