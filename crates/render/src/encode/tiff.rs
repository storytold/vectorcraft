//! TIFF export (baseline TIFF 6.0): little-endian (`II`) or big-endian (`MM`), strips of about
//! 8 KiB stored as they are, LZW-compressed (weezl, with TIFF's early code-size switch) or
//! PackBits; black and white, grey, RGB or CMYK (separated, ink amounts) samples with an optional
//! unassociated alpha channel, the resolution and an embedded colour profile. [`write`] is the one
//! TIFF writer: raster exports ([`encode`]) and EPS previews both use it.

use std::borrow::Cow;

use vectorcraft_color::cms;

use super::jpeg::{self, ColorModel};
use super::on_white;
use crate::Rendered;

/// The order of the bytes in the file's numbers (the samples are bytes either way).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ByteOrder {
    /// Least significant byte first (`II`).
    #[default]
    Little,
    /// Most significant byte first (`MM`).
    Big,
}

impl ByteOrder {
    pub const ALL: [ByteOrder; 2] = [ByteOrder::Little, ByteOrder::Big];

    pub fn id(self) -> &'static str {
        match self {
            ByteOrder::Little => "little",
            ByteOrder::Big => "big",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ByteOrder::Little => "Little-endian (II)",
            ByteOrder::Big => "Big-endian (MM)",
        }
    }

    /// By id or label, ignoring case; also by the file's marker (`ii`, `mm`) and the platforms the
    /// orders are known by (`ibm`, `mac`).
    pub fn from_id(s: &str) -> Option<Self> {
        let s = s.to_ascii_lowercase();
        match s.as_str() {
            "ii" | "ibm" | "pc" | "intel" => Some(ByteOrder::Little),
            "mm" | "mac" | "motorola" => Some(ByteOrder::Big),
            _ => Self::ALL.into_iter().find(|o| o.id() == s || o.label().eq_ignore_ascii_case(&s)),
        }
    }

    fn u16(self, v: u16) -> [u8; 2] {
        match self {
            ByteOrder::Little => v.to_le_bytes(),
            ByteOrder::Big => v.to_be_bytes(),
        }
    }

    fn u32(self, v: u32) -> [u8; 4] {
        match self {
            ByteOrder::Little => v.to_le_bytes(),
            ByteOrder::Big => v.to_be_bytes(),
        }
    }
}

/// How the strips are compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Lzw,
    /// Run lengths, each row on its own.
    PackBits,
}

impl Compression {
    fn code(self) -> u16 {
        match self {
            Compression::None => 1,
            Compression::Lzw => 5,
            Compression::PackBits => 32773,
        }
    }
}

/// What the samples mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Photometric {
    /// Grey or bilevel, 0 is white (a set bit is black).
    WhiteIsZero,
    /// Grey, 0 is black.
    BlackIsZero,
    Rgb,
    /// Ink amounts, 0 is no ink (CMYK).
    Separated,
}

impl Photometric {
    fn code(self) -> u16 {
        match self {
            Photometric::WhiteIsZero => 0,
            Photometric::BlackIsZero => 1,
            Photometric::Rgb => 2,
            Photometric::Separated => 5,
        }
    }
}

/// The pixels [`write`] stores.
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    /// Rows of samples, `bits` per sample, each row starting on a byte.
    pub data: &'a [u8],
    pub photometric: Photometric,
    /// Samples per pixel, the alpha one included.
    pub samples: u16,
    /// Bits per sample: 1 or 8.
    pub bits: u16,
    /// The last sample is unassociated alpha.
    pub alpha: bool,
}

/// How [`write`] stores the pixels and what else goes in the file.
pub struct Layout<'a> {
    pub byte_order: ByteOrder,
    pub compression: Compression,
    /// Pixels per inch.
    pub ppi: f64,
    /// A colour profile to embed.
    pub icc: Option<&'a [u8]>,
}

/// Uncompressed bytes per strip (the size the TIFF specification recommends).
const STRIP_BYTES: usize = 8 << 10;

/// A field's values.
enum Values {
    Ascii(Vec<u8>),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(u32, u32),
    Undefined(Vec<u8>),
}

impl Values {
    /// Field type and count.
    fn kind(&self) -> (u16, usize) {
        match self {
            Values::Ascii(v) => (2, v.len()),
            Values::Short(v) => (3, v.len()),
            Values::Long(v) => (4, v.len()),
            Values::Rational(..) => (5, 1),
            Values::Undefined(v) => (7, v.len()),
        }
    }

    /// Size in bytes.
    fn len(&self) -> usize {
        match self {
            Values::Ascii(v) | Values::Undefined(v) => v.len(),
            Values::Short(v) => v.len() * 2,
            Values::Long(v) => v.len() * 4,
            Values::Rational(..) => 8,
        }
    }

    fn bytes(&self, o: ByteOrder) -> Vec<u8> {
        match self {
            Values::Ascii(v) | Values::Undefined(v) => v.clone(),
            Values::Short(v) => v.iter().flat_map(|x| o.u16(*x)).collect(),
            Values::Long(v) => v.iter().flat_map(|x| o.u32(*x)).collect(),
            Values::Rational(n, d) => [o.u32(*n), o.u32(*d)].concat(),
        }
    }
}

/// Write `img` as a TIFF file: header, the image file directory, the values too long for it, then
/// the strips.
pub fn write(img: &Image, l: &Layout) -> Result<Vec<u8>, String> {
    let too_large = || "the image is too large for TIFF (at most 4 GB): lower the resolution".to_string();
    let (w, h) = (img.width as usize, img.height as usize);
    let row = (w * usize::from(img.samples) * usize::from(img.bits)).div_ceil(8);
    if w == 0 || h == 0 || !matches!(img.bits, 1 | 8) || img.samples == 0 || row.checked_mul(h) != Some(img.data.len()) {
        return Err("TIFF encoding failed: the pixel buffer doesn't match the image size".into());
    }
    let rows_per_strip = (STRIP_BYTES / row).clamp(1, h);
    let mut strips: Vec<u8> = Vec::with_capacity(img.data.len() / 2);
    let mut offsets = vec![];
    for strip in img.data.chunks(rows_per_strip * row) {
        offsets.push(strips.len());
        match l.compression {
            Compression::None => strips.extend_from_slice(strip),
            Compression::Lzw => strips.extend(craft_stream_codecs::lzw::encode(strip, true)),
            Compression::PackBits => strip.chunks(row).for_each(|r| packbits(r, &mut strips)),
        }
        // Strips start on a word boundary.
        if strips.len() % 2 == 1 {
            strips.push(0);
        }
    }
    let counts: Vec<usize> = offsets.iter().zip(offsets.iter().skip(1).chain([&strips.len()])).map(|(a, b)| b - a).collect();
    let mut fields: Vec<(u16, Values)> = vec![
        (256, Values::Long(vec![img.width])),
        (257, Values::Long(vec![img.height])),
        (258, Values::Short(vec![img.bits; usize::from(img.samples)])),
        (259, Values::Short(vec![l.compression.code()])),
        (262, Values::Short(vec![img.photometric.code()])),
        // StripOffsets: filled in once the directory's size is known.
        (273, Values::Long(vec![0; offsets.len()])),
        (277, Values::Short(vec![img.samples])),
        (278, Values::Long(vec![rows_per_strip as u32])),
        (279, Values::Long(counts.iter().map(|c| u32::try_from(*c).unwrap_or(u32::MAX)).collect())),
        (282, resolution(l.ppi)),
        (283, resolution(l.ppi)),
        // Chunky: the samples of a pixel together.
        (284, Values::Short(vec![1])),
        // Inches.
        (296, Values::Short(vec![2])),
        (305, Values::Ascii(b"VectorCraft\0".to_vec())),
    ];
    if img.photometric == Photometric::Separated {
        // InkSet: CMYK.
        fields.push((332, Values::Short(vec![1])));
    }
    if img.alpha {
        // Unassociated alpha.
        fields.push((338, Values::Short(vec![2])));
    }
    if let Some(icc) = l.icc {
        fields.push((34675, Values::Undefined(icc.to_vec())));
    }
    // Header, directory (count, 12-byte entries, next directory), longer values, strips.
    let dir_len = 2 + fields.len() * 12 + 4;
    let extra_len: usize = fields.iter().map(|(_, v)| v.len()).filter(|n| *n > 4).map(|n| n.next_multiple_of(2)).sum();
    let strip_at = 8 + dir_len + extra_len;
    let end = strip_at.checked_add(strips.len()).filter(|n| u32::try_from(*n).is_ok()).ok_or_else(too_large)?;
    if let Some((_, Values::Long(v))) = fields.iter_mut().find(|(tag, _)| *tag == 273) {
        *v = offsets.iter().map(|o| (strip_at + o) as u32).collect();
    }
    let o = l.byte_order;
    let mut out = Vec::with_capacity(end);
    out.extend_from_slice(if o == ByteOrder::Little { b"II" } else { b"MM" });
    out.extend(o.u16(42));
    out.extend(o.u32(8));
    out.extend(o.u16(fields.len() as u16));
    let mut extra = Vec::with_capacity(extra_len);
    for (tag, v) in &fields {
        let (ty, count) = v.kind();
        let mut bytes = v.bytes(o);
        out.extend(o.u16(*tag));
        out.extend(o.u16(ty));
        out.extend(o.u32(u32::try_from(count).map_err(|_| too_large())?));
        if bytes.len() <= 4 {
            // Left-justified in the value field.
            bytes.resize(4, 0);
            out.extend(bytes);
        } else {
            out.extend(o.u32((8 + dir_len + extra.len()) as u32));
            extra.extend(&bytes);
            if bytes.len() % 2 == 1 {
                extra.push(0);
            }
        }
    }
    out.extend(o.u32(0));
    out.extend(extra);
    out.extend(strips);
    Ok(out)
}

/// A resolution as a rational, to a hundredth of a pixel per inch.
fn resolution(ppi: f64) -> Values {
    let ppi = if ppi.is_finite() && ppi > 0.0 { ppi } else { 72.0 };
    if ppi.fract() == 0.0 && ppi <= u32::MAX as f64 {
        return Values::Rational(ppi as u32, 1);
    }
    Values::Rational((ppi * 100.0).round().clamp(1.0, u32::MAX as f64) as u32, 100)
}

/// `data` run-length encoded (PackBits: what TIFF's compression 32773 and PostScript's
/// `RunLengthDecode` read), without an end marker.
pub fn packbits(data: &[u8], out: &mut Vec<u8>) {
    craft_stream_codecs::runlength::encode_packbits_into(data, out);
}

/// The TIFF options of a raster export.
#[derive(Clone, Debug, PartialEq)]
pub struct TiffOptions {
    pub color_model: ColorModel,
    pub lzw: bool,
    pub byte_order: ByteOrder,
    /// Embed the colour profile of the colour model.
    pub embed_icc: bool,
}

impl Default for TiffOptions {
    fn default() -> Self {
        Self { color_model: ColorModel::Rgb, lzw: true, byte_order: ByteOrder::Little, embed_icc: true }
    }
}

/// `img` as a TIFF in `o.color_model`: RGB, with an alpha channel when some pixel isn't opaque,
/// grey on white (few readers take grey with alpha), or CMYK (its colours separated on white;
/// [`crate::Renderer::export_region`] draws inks).
pub fn encode(img: &Rendered, ppi: f64, o: &TiffOptions) -> Result<Vec<u8>, String> {
    let px = match o.color_model {
        ColorModel::Cmyk => jpeg::separated(img),
        ColorModel::Rgb | ColorModel::Gray => img.to_straight(),
    };
    encode_samples(&px, img.width, img.height, ppi, o)
}

/// Pixels as a TIFF in `o.color_model` (see [`encode`]): straight RGBA for RGB and grey, ink
/// amounts for CMYK.
pub(crate) fn encode_samples(px: &[u8], width: u32, height: u32, ppi: f64, o: &TiffOptions) -> Result<Vec<u8>, String> {
    let model = o.color_model;
    let (data, photometric, alpha) = match model {
        ColorModel::Cmyk => (Cow::Borrowed(px), Photometric::Separated, false),
        ColorModel::Rgb => {
            let rgba = px.as_chunks::<4>().0;
            let alpha = rgba.iter().any(|p| p[3] < 255);
            let data = if alpha { Cow::Borrowed(px) } else { rgba.iter().flat_map(|p| [p[0], p[1], p[2]]).collect() };
            (data, Photometric::Rgb, alpha)
        }
        ColorModel::Gray => {
            let gray = jpeg::to_gray();
            (px.as_chunks::<4>().0.iter().map(|p| gray(on_white(p))).collect(), Photometric::BlackIsZero, false)
        }
    };
    let icc = if o.embed_icc { Some(cms::icc_bytes(&model.profile()).map_err(|e| format!("TIFF colour profile: {e}"))?) } else { None };
    let samples = (model.channels() + usize::from(alpha)) as u16;
    let image = Image { width, height, data: &data, photometric, samples, bits: 8, alpha };
    let compression = if o.lzw { Compression::Lzw } else { Compression::None };
    write(&image, &Layout { byte_order: o.byte_order, compression, ppi, icc: icc.as_deref() })
}
