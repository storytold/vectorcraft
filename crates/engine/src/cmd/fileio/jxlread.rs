//! Reading JPEG XL images (`.jxl`, ISO/IEC 18181) for File › Open and File › Place, with the
//! jxl-oxide decoder: the bare codestream and the ISOBMFF container. The first frame of an
//! animation is read, with the orientation its header gives applied.
//!
//! Colors come back as sRGB whenever the decoder converts them without a color management
//! system: XYB-encoded images, every color space a JPEG XL header names, and embedded ICC
//! profiles that describe one of them. Any other image with an embedded ICC profile comes back
//! with its samples as stored, as PNG and JPEG files with an embedded profile are drawn, and with
//! the profile when it is an RGB one. Gray images with an ICC profile the decoder cannot convert
//! are refused. 16-bit and floating-point samples become 8 bits, alpha comes back straight,
//! and CMYK images are refused. The file is untrusted: the decoder is given only a container's
//! image boxes, the image's size is checked from its header before any pixels are decoded, and the
//! decoder's image buffers are capped.

use jxl_color::ColorTransform;
use jxl_oxide::color::{ColourEncoding, TransferFunction};
use jxl_oxide::{AllocTracker, ColorEncodingWithProfile, EnumColourEncoding, HdrType, InitializeResult, JxlImage, NullCms, RenderingIntent};

/// A bare JPEG XL codestream starts with these bytes.
const CODESTREAM: [u8; 2] = [0xff, 0x0a];
/// The signature box an ISOBMFF JPEG XL container starts with.
const CONTAINER: [u8; 12] = [0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a];
/// How many bytes the decoder is given first to read the header. Each later try gives it twice as
/// many bytes, and each try decodes the embedded ICC profile from its start again.
const HEAD: usize = 4096;
/// The most the decoder's buffers may take while it reads the header. Its `AllocTracker` counts the
/// image buffers only: an embedded ICC profile is decoded into buffers of its own, which jxl-color
/// limits to 256 MiB.
const HEADER_BUFFERS: usize = 16 << 20;
/// The most the decoder's buffers may take for the pixels: this many times the image's size as
/// RGBA, plus [`HEADER_BUFFERS`], and never more than the `max_alloc` of [`decode`], the limit the
/// `image` crate's decoders get in `raster_image`. The decoder keeps each channel as 32-bit
/// samples: a lossless 16-bit RGBA image of 1024 × 768 pixels took 12 times its size as RGBA.
const DECODER_BUFFERS: u64 = 16;

const TRUNCATED: &str = "the JPEG XL file is cut short";

/// Is `bytes` a JPEG XL image (a bare codestream or a container)?
pub fn is_jxl(bytes: &[u8]) -> bool {
    bytes.starts_with(&CODESTREAM) || bytes.starts_with(&CONTAINER)
}

/// Resolution after the decoder applies the image header's orientation.
pub(crate) fn oriented_resolution(ppi: Option<(f64, f64)>, orientation: u32) -> Option<(f64, f64)> {
    ppi.map(|(x, y)| if (5..=8).contains(&orientation) { (y, x) } else { (x, y) })
}

/// The first frame of a JPEG XL image as straight RGBA, with the embedded ICC profile its colors
/// are in when they are not converted to sRGB ([`output_profile`]), and the header orientation.
/// Fails, without decoding the
/// pixels, when the image would take more than `max_alloc` bytes as RGBA. The image is also too
/// large to draw when the decoder's buffers for its pixels need more than they are allowed. A
/// container whose selected boxes exceed `max_alloc` or [`HEADER_BUFFERS`], whichever is greater,
/// is refused with "the JPEG XL container is too large to read".
pub fn decode(bytes: &[u8], max_alloc: u64) -> Result<(image::RgbaImage, Option<Vec<u8>>, u32), String> {
    let pixel_boxes = pixel_boxes(bytes, max_alloc.max(HEADER_BUFFERS as u64))?;
    let bytes = pixel_boxes.as_deref().unwrap_or(bytes);
    // The decoder reads the header first, with buffers for the header alone.
    let (head, _) = header(bytes, HEADER_BUFFERS, None)?;
    let (w, h) = (head.width(), head.height());
    let rgba = u64::from(w) * u64::from(h) * 4;
    if rgba > max_alloc {
        return Err(too_large(w, h));
    }
    if head.pixel_format().has_black() {
        return Err("CMYK JPEG XL images can't be read".into());
    }
    // Then it reads the file again until the first frame is loaded, with buffers for the pixels.
    let budget = rgba.saturating_mul(DECODER_BUFFERS).saturating_add(HEADER_BUFFERS as u64).min(max_alloc);
    let budget = usize::try_from(budget).unwrap_or(usize::MAX);
    let size = Some((w, h));
    let (mut image, mut at) = header(bytes, budget, size)?;
    let mut end = at;
    while image.num_loaded_keyframes() == 0 {
        if end == bytes.len() {
            return Err(TRUNCATED.into());
        }
        end = end.saturating_mul(2).max(HEAD).min(bytes.len());
        at += image.feed_bytes(&bytes[at..end]).map_err(|e| failed(e, size))?;
    }
    let metadata = &image.image_header().metadata;
    let associated = metadata.alpha().and_then(|i| metadata.ec_info.get(i)).and_then(|ec| ec.alpha_associated()) == Some(true);
    let intent = RenderingIntent::Relative;
    let srgb = if metadata.grayscale() { EnumColourEncoding::gray_srgb(intent) } else { EnumColourEncoding::srgb(intent) };
    let profile = output_profile(metadata.xyb_encoded, metadata.grayscale(), image.original_icc())?;
    let orientation = metadata.orientation;
    // Premultiplied colors are divided by alpha in the encoding they are stored in, and converted
    // to sRGB after that. XYB-encoded images come from the decoder in linear sRGB (tone mapped when
    // they are HDR), which the sRGB transfer curve alone turns into sRGB.
    let (request, convert) = match (to_srgb_without_cms(metadata.xyb_encoded, image.original_icc()), associated) {
        (false, _) => (None, None),
        (true, false) => (Some(srgb), None),
        (true, true) if metadata.xyb_encoded => (Some(EnumColourEncoding { tf: TransferFunction::Linear, ..srgb }), Some(ToSrgb::Curve)),
        (true, true) => {
            let stored = match (&metadata.colour_encoding, image.original_icc()) {
                (ColourEncoding::Enum(e), _) => ColorEncodingWithProfile::new(e.clone()),
                (_, Some(icc)) => ColorEncodingWithProfile::with_icc(icc).map_err(unreadable)?,
                (_, None) => return Err(unreadable("no color encoding")),
            };
            let mut builder = ColorTransform::builder();
            builder.from_pq(image.hdr_type() == Some(HdrType::Pq));
            let to = ColorEncodingWithProfile::new(srgb);
            let t = builder.build(&stored, &to, &metadata.opsin_inverse_matrix, &metadata.tone_mapping, &NullCms).map_err(unreadable)?;
            (None, Some(ToSrgb::Transform(t)))
        }
    };
    if let Some(encoding) = request {
        image.request_color_encoding(encoding);
    }
    let render = image.render_frame(0).map_err(|e| failed(e, size))?;
    let mut stream = render.stream();
    let channels = stream.channels() as usize;
    if !(1..=4).contains(&channels) {
        return Err(unreadable(format!("{channels} channels")));
    }
    let mut out = image::RgbaImage::new(stream.width(), stream.height());
    let width = out.width() as usize;
    let mut row = vec![0f32; width * channels];
    let mut colors = [vec![0f32; width], vec![0f32; width], vec![0f32; width]];
    for line in out.chunks_exact_mut((width * 4).max(4)) {
        if stream.write_to_buffer(&mut row) != row.len() {
            return Err(TRUNCATED.into());
        }
        to_rgba(&row, channels, associated, convert.as_ref(), &mut colors, line)?;
    }
    Ok((out, profile, orientation))
}

/// How colors whose alpha was divided out become sRGB.
pub(crate) enum ToSrgb {
    /// A conversion from the encoding the colors are stored in.
    Transform(ColorTransform),
    /// The sRGB transfer curve, for colors in linear sRGB.
    Curve,
}

/// One row of samples, `channels` to a pixel (gray or RGB, then alpha when there are 2 or 4), as
/// straight RGBA bytes in `out`: colors divided by alpha when it is `associated`, then converted
/// with `convert`. `colors` holds a row of red, green and blue; a gray sample goes into all three,
/// as jxl-render's `clone_gray` does before its own conversions.
pub(crate) fn to_rgba(
    row: &[f32],
    channels: usize,
    associated: bool,
    convert: Option<&ToSrgb>,
    colors: &mut [Vec<f32>; 3],
    out: &mut [u8],
) -> Result<(), String> {
    let alpha = |s: &[f32]| if matches!(channels, 2 | 4) { s.last().map_or(1.0, |a| a.clamp(0.0, 1.0)) } else { 1.0 };
    let gray = channels < 3;
    for (x, s) in row.chunks_exact(channels).enumerate() {
        let a = alpha(s);
        for (i, plane) in colors.iter_mut().enumerate() {
            let v = s.get(if gray { 0 } else { i }).copied().unwrap_or(0.0);
            if let Some(c) = plane.get_mut(x) {
                *c = if associated && a > 0.0 { v / a } else { v };
            }
        }
    }
    // The conversion leaves gray in the first row when it gives one channel.
    let gray = match convert {
        Some(ToSrgb::Transform(t)) => {
            let mut planes: Vec<&mut [f32]> = colors.iter_mut().map(Vec::as_mut_slice).collect();
            t.run(&mut planes).map_err(unreadable)? < 3
        }
        Some(ToSrgb::Curve) => {
            colors.iter_mut().flatten().for_each(|v| *v = if *v <= 0.003_130_8 { *v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 });
            gray
        }
        None => gray,
    };
    for (x, (px, s)) in out.as_chunks_mut::<4>().0.iter_mut().zip(row.chunks_exact(channels)).enumerate() {
        let c = |i: usize| colors.get(if gray { 0 } else { i }).and_then(|p| p.get(x)).copied().unwrap_or(0.0);
        *px = [byte(c(0)), byte(c(1)), byte(c(2)), byte(alpha(s))];
    }
    Ok(())
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

/// A decoder whose buffers may take `buffers` bytes, given as few of `bytes` as it needs to read
/// the header → (the decoder, how many bytes it took). `size` is the image's size when it is
/// known, for the error when the buffers need more ([`failed`]).
fn header(bytes: &[u8], buffers: usize, size: Option<(u32, u32)>) -> Result<(JxlImage, usize), String> {
    let mut uninit = JxlImage::builder().alloc_tracker(AllocTracker::with_limit(buffers)).build_uninit();
    let (mut at, mut end) = (0, 0usize);
    loop {
        end = end.saturating_mul(2).max(HEAD).min(bytes.len());
        at += uninit.feed_bytes(&bytes[at..end]).map_err(|e| failed(e, size))?;
        match uninit.try_init().map_err(|e| failed(e, size))? {
            InitializeResult::Initialized(image) => return Ok((image, at)),
            InitializeResult::NeedMoreData(_) if end == bytes.len() => return Err(TRUNCATED.into()),
            InitializeResult::NeedMoreData(more) => uninit = more,
        }
    }
}

/// Does the decoder convert to sRGB without a color management system? It does for XYB-encoded
/// images and the color spaces a header names; for an embedded ICC profile, only when the profile
/// describes one of those.
fn to_srgb_without_cms(xyb: bool, icc: Option<&[u8]>) -> bool {
    xyb || icc.is_none_or(|icc| ColorEncodingWithProfile::with_icc(icc).is_ok_and(|p| p.icc_profile().is_empty()))
}

/// The embedded RGB profile when the decoder leaves colors in it. Gray colors without a
/// conversion to sRGB cannot be stored as RGB without changing their profile.
pub(crate) fn output_profile(xyb: bool, grayscale: bool, icc: Option<&[u8]>) -> Result<Option<Vec<u8>>, String> {
    let converts = to_srgb_without_cms(xyb, icc);
    if grayscale && !converts {
        return Err(unreadable("unsupported grayscale ICC profile"));
    }
    Ok(icc.filter(|icc| icc.get(16..20) == Some(b"RGB ") && !converts).map(<[u8]>::to_vec))
}

/// The decoder's error `e` as a message. When the decoder's buffers needed more than they were
/// allowed and the image's `size` is known, the image is too large to draw. Otherwise the file
/// can't be read.
fn failed(e: Box<dyn std::error::Error + Send + Sync>, size: Option<(u32, u32)>) -> String {
    let first: &(dyn std::error::Error + 'static) = &*e;
    let out_of_memory = std::iter::successors(Some(first), |c| c.source()).any(|c| c.is::<jxl_grid::OutOfMemory>());
    match size {
        Some((w, h)) if out_of_memory => too_large(w, h),
        _ => unreadable(e),
    }
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
