//! Reading Photoshop documents (PSD, and PSB, the large document format) for File › Open and
//! File › Place (#918): the merged image, the flattened picture every PSD carries after its layers,
//! as a linked PSD shows. Written from Adobe's public "Photoshop File Formats Specification"
//! (<https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/>). Layers aren't read.
//!
//! Every mode Photoshop saves a merged image in is read: Bitmap, Grayscale, Duotone (as its
//! grayscale data), Indexed, RGB, CMYK and Lab, at 1, 8, 16 and 32 bits per channel, stored raw,
//! PackBits-compressed or zipped. The merged image of a document with transparency is stored on
//! white with its alpha in the first extra channel: it comes back with straight colours. The file
//! is untrusted: every read is checked and the image's size is capped before anything is
//! allocated.

use std::io::Read;

use vectorcraft_color::cms::{Lab, lab::lab_to_srgb};

/// Photoshop's colour modes (the header's numbers), as far as the merged image is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Bitmap,
    Gray,
    Indexed,
    Rgb,
    Cmyk,
    Lab,
}

impl Mode {
    /// The channels holding colour (an alpha channel may follow them).
    fn colors(self) -> usize {
        match self {
            Mode::Bitmap | Mode::Gray | Mode::Indexed => 1,
            Mode::Rgb | Mode::Lab => 3,
            Mode::Cmyk => 4,
        }
    }
}

/// Is `bytes` a Photoshop document (PSD or PSB)?
pub fn is_psd(bytes: &[u8]) -> bool {
    bytes.starts_with(b"8BPS") && matches!(bytes.get(4..6), Some([0, 1] | [0, 2]))
}

/// Is `bytes` a PSB, Photoshop's large document format?
pub fn is_psb(bytes: &[u8]) -> bool {
    is_psd(bytes) && bytes.get(4..6) == Some(&[0, 2])
}

const TRUNCATED: &str = "the PSD file is cut short";
const DAMAGED: &str = "the PSD file's header is damaged";

/// Big-endian reads through `b`, each checked against its end.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.at.checked_add(n).and_then(|end| self.b.get(self.at..end)).ok_or(TRUNCATED)?;
        self.at += n;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.take(N)?.try_into().map_err(|_| TRUNCATED.to_string())
    }
    fn u8(&mut self) -> Result<u8, String> {
        self.array::<1>().map(|[v]| v)
    }
    fn u16(&mut self) -> Result<u16, String> {
        self.array().map(u16::from_be_bytes)
    }
    fn u32(&mut self) -> Result<u32, String> {
        self.array().map(u32::from_be_bytes)
    }
    /// A section length: 4 bytes in a PSD, 8 in a PSB.
    fn length(&mut self, psb: bool) -> Result<usize, String> {
        let n = if psb { self.array().map(u64::from_be_bytes)? } else { u64::from(self.u32()?) };
        usize::try_from(n).map_err(|_| TRUNCATED.to_string())
    }
    /// A section: its length, then that many bytes.
    fn section(&mut self, psb: bool) -> Result<&'a [u8], String> {
        let n = self.length(psb)?;
        self.take(n)
    }
    fn rest(&self) -> &'a [u8] {
        self.b.get(self.at..).unwrap_or_default()
    }
}

/// The fixed-size header and the sections before the image data.
struct Header<'a> {
    psb: bool,
    channels: usize,
    width: u32,
    height: u32,
    depth: u16,
    mode: Mode,
    /// Colour mode data: an indexed image's palette (256 reds, then greens, then blues).
    palette: &'a [u8],
    /// The image resources.
    resources: &'a [u8],
    /// The layer and mask information.
    layers: &'a [u8],
}

fn header<'a>(r: &mut Reader<'a>) -> Result<Header<'a>, String> {
    if !is_psd(r.b) {
        return Err("not a PSD file".into());
    }
    r.take(4)?;
    let psb = r.u16()? == 2;
    r.take(6)?;
    let channels = usize::from(r.u16()?);
    let height = r.u32()?;
    let width = r.u32()?;
    let depth = r.u16()?;
    let mode = match r.u16()? {
        0 => Mode::Bitmap,
        // Duotone documents keep their grayscale data in the merged image.
        1 | 8 => Mode::Gray,
        2 => Mode::Indexed,
        3 => Mode::Rgb,
        4 => Mode::Cmyk,
        9 => Mode::Lab,
        7 => return Err("Multichannel PSD files can't be read: save it in RGB, CMYK or Grayscale mode".into()),
        m => return Err(format!("the PSD file has an unknown colour mode ({m})")),
    };
    let max_side = if psb { 300_000 } else { 30_000 };
    if !(1..=56).contains(&channels) || channels < mode.colors() || !(1..=max_side).contains(&width) || !(1..=max_side).contains(&height) {
        return Err(DAMAGED.into());
    }
    let depth_ok = match mode {
        Mode::Bitmap => depth == 1,
        Mode::Indexed => depth == 8,
        Mode::Cmyk | Mode::Lab => matches!(depth, 8 | 16),
        Mode::Gray | Mode::Rgb => matches!(depth, 8 | 16 | 32),
    };
    if !depth_ok {
        return Err(format!("{depth}-bit PSD files in this colour mode can't be read"));
    }
    let palette = r.section(false)?;
    let resources = r.section(false)?;
    let layers = r.section(psb)?;
    Ok(Header { psb, channels, width, height, depth, mode, palette, resources, layers })
}

/// The resolution a Photoshop document declares (pixels per inch, across and down), from its
/// ResolutionInfo resource (1005).
pub fn resolution(bytes: &[u8]) -> Option<(f64, f64)> {
    let mut r = Reader::new(bytes);
    let h = header(&mut r).ok()?;
    let mut r = Reader::new(h.resources);
    while !r.rest().is_empty() {
        r.take(4).ok()?;
        let id = r.u16().ok()?;
        // A Pascal string padded to an even size, length byte included.
        let name = usize::from(r.u8().ok()?);
        r.take(name + (name + 1) % 2).ok()?;
        let size = usize::try_from(r.u32().ok()?).ok()?;
        let data = r.take(size).ok()?;
        r.take(size % 2).ok()?;
        if id == 0x03ed {
            let mut d = Reader::new(data);
            // Fixed 16.16 values, each with its unit (1: per inch, 2: per centimetre) and the
            // unit the size is shown in.
            let mut axis = || -> Option<f64> {
                let v = f64::from(d.u32().ok()?) / 65536.0;
                let unit = d.u16().ok()?;
                d.u16().ok()?;
                Some(if unit == 2 { v * 2.54 } else { v })
            };
            return Some((axis()?, axis()?));
        }
    }
    None
}

/// Does the merged image keep its transparency in the first extra channel? Yes when the layer
/// count is negative: in the layer info, or for 16- and 32-bit documents (whose layer info is
/// empty) in their `Lr16` / `Lr32` block.
fn merged_alpha(layers: &[u8], psb: bool) -> bool {
    let count_negative = |info: &[u8]| info.get(..2).is_some_and(|c| i16::from_be_bytes([c[0], c[1]]) < 0);
    let mut r = Reader::new(layers);
    let Ok(info) = r.section(psb) else { return false };
    if !info.is_empty() {
        return count_negative(info);
    }
    // The global layer mask info, then tagged blocks: signature, key, length (8 bytes for the
    // layer blocks of a PSB), data padded to 4 bytes.
    if r.section(false).is_err() {
        return false;
    }
    while r.rest().len() >= 12 {
        let (Ok(_), Ok(key)) = (r.take(4), r.array::<4>()) else { return false };
        let layer_block = matches!(&key, b"Lr16" | b"Lr32" | b"Layr");
        let long = layer_block || matches!(&key, b"LMsk" | b"Mt16" | b"Mt32" | b"Mtrn" | b"Alph" | b"FMsk" | b"lnk2" | b"FEid" | b"FXid" | b"PxSD");
        let Ok(data) = r.section(psb && long) else {
            return false;
        };
        if layer_block {
            return count_negative(data);
        }
        if r.take((4 - data.len() % 4) % 4).is_err() {
            return false;
        }
    }
    false
}

/// The most bytes one channel row may take (a PSB at 32 bits).
fn row_bytes(width: usize, depth: u16) -> usize {
    if depth == 1 { width.div_ceil(8) } else { width * usize::from(depth / 8) }
}

/// Undo PackBits: `src` into `dst` (a row; a short or overlong run leaves the rest as it is).
fn unpack_bits(mut src: &[u8], dst: &mut [u8]) {
    let mut at = 0;
    while let Some((&n, rest)) = src.split_first() {
        src = rest;
        let n = n as i8;
        if n >= 0 {
            let len = usize::from(n.unsigned_abs()) + 1;
            let (run, rest) = src.split_at(len.min(src.len()));
            src = rest;
            for &b in run {
                if let Some(d) = dst.get_mut(at) {
                    *d = b;
                }
                at += 1;
            }
        } else if n != -128 {
            let Some((&b, rest)) = src.split_first() else { break };
            src = rest;
            let len = usize::from(n.unsigned_abs()) + 1;
            for d in dst.iter_mut().skip(at).take(len) {
                *d = b;
            }
            at += len;
        }
        if at >= dst.len() {
            break;
        }
    }
}

/// The first `channels` channels of the image data (after its compression method), each as
/// `height` rows of `row` bytes, one after the other.
fn channel_data(data: &[u8], h: &Header<'_>, channels: usize, row: usize) -> Result<Vec<u8>, String> {
    let (rows, all) = (h.height as usize, h.channels);
    let mut r = Reader::new(data);
    let method = r.u16()?;
    let wanted = channels * rows * row;
    match method {
        0 => r.take(wanted).map(<[u8]>::to_vec),
        1 => {
            // Every row's byte count (4 bytes each in a PSB), channel by channel: those of the
            // channels read, then the others'.
            let size = if h.psb { 4 } else { 2 };
            let counts = r.take(channels * rows * size)?;
            r.take((all - channels) * rows * size)?;
            let counts = counts.chunks_exact(size).map(|c| c.iter().fold(0usize, |n, &b| n << 8 | usize::from(b)));
            let mut out = vec![0; wanted];
            for (dst, n) in out.chunks_exact_mut(row).zip(counts) {
                unpack_bits(r.take(n)?, dst);
            }
            Ok(out)
        }
        2 | 3 => {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(r.rest())
                .take(wanted as u64)
                .read_to_end(&mut out)
                .map_err(|e| format!("the PSD file's image data can't be read: {e}"))?;
            if out.len() < wanted {
                return Err(TRUNCATED.into());
            }
            if method == 3 {
                // Each row stores the difference from the sample before it.
                match h.depth {
                    8 => out.chunks_exact_mut(row).for_each(|r| (1..r.len()).for_each(|i| r[i] = r[i].wrapping_add(r[i - 1]))),
                    16 => out.chunks_exact_mut(row).for_each(|r| {
                        let mut prev = 0u16;
                        for s in r.as_chunks_mut::<2>().0 {
                            prev = prev.wrapping_add(u16::from_be_bytes(*s));
                            *s = prev.to_be_bytes();
                        }
                    }),
                    _ => return Err("zipped 32-bit PSD image data with prediction can't be read".into()),
                }
            }
            Ok(out)
        }
        m => Err(format!("the PSD file's image data uses an unknown compression ({m})")),
    }
}

/// sRGB's transfer curve, for 32-bit documents, whose values are linear.
fn srgb_encode(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Sample `x` of a channel row, 0..1.
fn sample(row: &[u8], x: usize, depth: u16, linear: bool) -> f32 {
    match depth {
        1 => {
            // Bitmap: a set bit is black.
            let bit = row.get(x / 8).map_or(0, |b| (b >> (7 - x % 8)) & 1);
            if bit == 1 { 0.0 } else { 1.0 }
        }
        16 => row.get(x * 2..x * 2 + 2).map_or(0.0, |s| f32::from(u16::from_be_bytes([s[0], s[1]])) / 65535.0),
        32 => {
            let v = row.get(x * 4..x * 4 + 4).map_or(0.0, |s| f32::from_be_bytes([s[0], s[1], s[2], s[3]]));
            let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            if linear { srgb_encode(v) } else { v }
        }
        _ => row.get(x).map_or(0.0, |&b| f32::from(b) / 255.0),
    }
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A Photoshop document's merged image as straight RGBA. CMYK converts through `cmyk` (ink
/// amounts 0..1 → sRGB). Fails, without allocating, when the image would need more than
/// `max_alloc` bytes.
pub fn decode(bytes: &[u8], max_alloc: u64, cmyk: impl Fn([f32; 4]) -> [f32; 3]) -> Result<image::RgbaImage, String> {
    let mut r = Reader::new(bytes);
    let h = header(&mut r)?;
    let colors = h.mode.colors();
    let alpha = h.channels > colors && merged_alpha(h.layers, h.psb);
    let channels = colors + usize::from(alpha);
    let (w, rows) = (h.width as usize, h.height as usize);
    let row = row_bytes(w, h.depth);
    // The channels read and the RGBA image made of them.
    let need = (channels as u64 * row as u64).saturating_add(4 * w as u64).saturating_mul(rows as u64);
    if need > max_alloc {
        return Err(format!("the PSD image is too large to read ({} × {} pixels)", h.width, h.height));
    }
    let data = channel_data(r.rest(), &h, channels, row)?;
    let mut out = image::RgbaImage::new(h.width, h.height);
    let linear = h.depth == 32;
    for (y, line) in out.rows_mut().enumerate() {
        let planes: Vec<&[u8]> = (0..channels).map(|c| data.get((c * rows + y) * row..(c * rows + y + 1) * row).unwrap_or_default()).collect();
        for (x, px) in line.enumerate() {
            let v = |c: usize| planes.get(c).map_or(0.0, |p| sample(p, x, h.depth, linear && c < colors));
            let rgb = match h.mode {
                Mode::Bitmap | Mode::Gray => [v(0); 3],
                Mode::Indexed => {
                    let i = usize::from(byte(v(0)));
                    [0, 256, 512].map(|at| h.palette.get(at + i).map_or(0.0, |&c| f32::from(c) / 255.0))
                }
                Mode::Rgb => [v(0), v(1), v(2)],
                // Inks are stored as 1 − amount.
                Mode::Cmyk => cmyk([0, 1, 2, 3].map(|c| 1.0 - v(c))),
                Mode::Lab => lab_to_srgb(Lab { l: v(0) * 100.0, a: v(1) * 255.0 - 128.0, b: v(2) * 255.0 - 128.0 }),
            };
            let a = if alpha { v(colors) } else { 1.0 };
            // The merged image is stored on white: take the white back out.
            let straight = |c: f32| if a <= 0.0 { 0.0 } else { (c - (1.0 - a)) / a };
            let rgb = if alpha { rgb.map(straight) } else { rgb };
            px.0 = [byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(a)];
        }
    }
    Ok(out)
}
