//! Reading Photoshop documents (PSD, and PSB, the large document format) for File › Open and
//! File › Place (#918): the merged image, the flattened picture every PSD carries after its layers,
//! as a linked PSD shows. Written from Adobe's public "Photoshop File Formats Specification"
//! (<https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/>). File › Open reads the
//! layers with [`super::psdlayers`], which shares this reader. The clipping path a document names
//! ([`clipping_path`]) clips its merged image where it opens or is placed.
//!
//! Every mode Photoshop saves a merged image in is read: Bitmap, Grayscale, Duotone (as its
//! grayscale data), Indexed, RGB, CMYK and Lab, at 1, 8, 16 and 32 bits per channel, stored raw,
//! PackBits-compressed or zipped. The merged image of a document with transparency is stored on
//! white with its alpha in the first extra channel: it comes back with straight colours. The file
//! is untrusted: every read is checked and the image's size is capped before anything is
//! allocated.

use std::io::Read;

use vectorcraft_color::cms::{Lab, lab::lab_to_srgb};
use vectorcraft_geom::{Anchor, AnchorKind, PathData, Point, SubPath};

/// Photoshop's colour modes (the header's numbers), as far as the merged image is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Bitmap,
    Gray,
    Indexed,
    Rgb,
    Cmyk,
    Lab,
}

impl Mode {
    /// The channels holding colour (an alpha channel may follow them).
    pub(super) fn colors(self) -> usize {
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

pub(super) const TRUNCATED: &str = "the PSD file is cut short";
const DAMAGED: &str = "the PSD file's header is damaged";

/// Big-endian reads through `b`, each checked against its end.
pub(super) struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub(super) fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.at.checked_add(n).and_then(|end| self.b.get(self.at..end)).ok_or(TRUNCATED)?;
        self.at += n;
        Ok(s)
    }
    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.take(N)?.try_into().map_err(|_| TRUNCATED.to_string())
    }
    pub(super) fn u8(&mut self) -> Result<u8, String> {
        self.array::<1>().map(|[v]| v)
    }
    pub(super) fn u16(&mut self) -> Result<u16, String> {
        self.array().map(u16::from_be_bytes)
    }
    pub(super) fn u32(&mut self) -> Result<u32, String> {
        self.array().map(u32::from_be_bytes)
    }
    /// A section length: 4 bytes in a PSD, 8 in a PSB.
    pub(super) fn length(&mut self, psb: bool) -> Result<usize, String> {
        let n = if psb { self.array().map(u64::from_be_bytes)? } else { u64::from(self.u32()?) };
        usize::try_from(n).map_err(|_| TRUNCATED.to_string())
    }
    /// A section: its length, then that many bytes.
    pub(super) fn section(&mut self, psb: bool) -> Result<&'a [u8], String> {
        let n = self.length(psb)?;
        self.take(n)
    }
    pub(super) fn rest(&self) -> &'a [u8] {
        self.b.get(self.at..).unwrap_or_default()
    }
}

/// The fixed-size header and the sections before the image data.
pub(super) struct Header<'a> {
    pub(super) psb: bool,
    channels: usize,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) depth: u16,
    pub(super) mode: Mode,
    /// Colour mode data: an indexed image's palette (256 reds, then greens, then blues).
    palette: &'a [u8],
    /// The image resources.
    pub(super) resources: &'a [u8],
    /// The layer and mask information.
    pub(super) layers: &'a [u8],
}

pub(super) fn header<'a>(r: &mut Reader<'a>) -> Result<Header<'a>, String> {
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
    let (_, _, data) = resources(h.resources, false).find(|(id, _, _)| *id == 0x03ed)?;
    let mut d = Reader::new(data);
    // Fixed 16.16 values, each with its unit (1: per inch, 2: per centimetre) and the unit the
    // size is shown in.
    let mut axis = || -> Option<f64> {
        let v = f64::from(d.u32().ok()?) / 65536.0;
        let unit = d.u16().ok()?;
        d.u16().ok()?;
        Some(if unit == 2 { v * 2.54 } else { v })
    };
    Some((axis()?, axis()?))
}

/// The image resource blocks of `resources`, in file order: each one's id, name (the bytes of its
/// Pascal string) and data. Stops at the first block that is cut short. With `signed`, only the
/// blocks whose signature is `8BIM` are given.
pub(super) fn resources(resources: &[u8], signed: bool) -> impl Iterator<Item = (u16, &[u8], &[u8])> {
    let mut r = Reader::new(resources);
    std::iter::from_fn(move || if r.rest().is_empty() { None } else { block(&mut r).ok() })
        .filter(move |(ours, ..)| *ours || !signed)
        .map(|(_, id, name, data)| (id, name, data))
}

/// The data of the first `8BIM` image resource block `id` of `resources`. `None` when there is
/// none before the blocks end or before a block that is cut short ahead of it. An error when
/// that block itself is cut short.
pub(super) fn resource(resources: &[u8], id: u16) -> Result<Option<&[u8]>, String> {
    let mut r = Reader::new(resources);
    while !r.rest().is_empty() {
        match block(&mut r) {
            Ok((true, i, _, data)) if i == id => return Ok(Some(data)),
            Ok(_) => {}
            Err(Some((true, i))) if i == id => return Err(TRUNCATED.into()),
            Err(_) => return Ok(None),
        }
    }
    Ok(None)
}

/// An image resource block: whether its signature is `8BIM`, its id, its name (the bytes of its
/// Pascal string) and its data.
type Block<'a> = (bool, u16, &'a [u8], &'a [u8]);

/// The image resource block that `r` starts with. When it is cut short, the error gives whether
/// its signature is `8BIM` and its id, when those were read.
fn block<'a>(r: &mut Reader<'a>) -> Result<Block<'a>, Option<(bool, u16)>> {
    let ours = r.take(4).map_err(|_| None)? == b"8BIM";
    let id = r.u16().map_err(|_| None)?;
    let mut rest = || -> Result<(&'a [u8], &'a [u8]), String> {
        // A Pascal string padded to an even size, length byte included.
        let len = usize::from(r.u8()?);
        let name = r.take(len)?;
        r.take((len + 1) % 2)?;
        let size = usize::try_from(r.u32()?).map_err(|_| TRUNCATED.to_string())?;
        let data = r.take(size)?;
        r.take(size % 2)?;
        Ok((name, data))
    };
    let (name, data) = rest().map_err(|_| Some((ours, id)))?;
    Ok((ours, id, name, data))
}

/// The resource naming the document's clipping path.
const CLIPPING_PATH_NAME: u16 = 2999;

/// The saved paths' resource ids.
const SAVED_PATHS: std::ops::RangeInclusive<u16> = 2000..=2997;

/// The clipping path a PSD or PSB file names, in its pixel space (x across its width, y down
/// its height): the saved path (resources 2000 to 2997) whose name the clipping path resource
/// (2999) gives, both `8BIM` resources. `None` when the document names none, names a path it doesn't have, or that path
/// has no anchors or more records than [`vectorcraft_geom::path::MAX_READ_ANCHORS`]. Saved paths
/// use the even-odd rule. A path whose initial fill record says the fill starts with all pixels
/// begins with a subpath around the whole image, so its subpaths cut holes in it.
pub fn clipping_path(bytes: &[u8]) -> Option<PathData> {
    let mut r = Reader::new(bytes);
    let h = header(&mut r).ok()?;
    let (_, _, named) = resources(h.resources, true).find(|(id, _, _)| *id == CLIPPING_PATH_NAME)?;
    // A Pascal string, then the flatness and a fill rule, which the specification says is ignored.
    let len = usize::from(*named.first()?);
    let name = named.get(1..1 + len)?;
    let (_, _, data) = resources(h.resources, true).find(|(id, n, _)| SAVED_PATHS.contains(id) && *n == name)?;
    // A path of more records than a document may hold anchors is left out before it is built.
    if data.len() / PATH_RECORD > vectorcraft_geom::path::MAX_READ_ANCHORS {
        return None;
    }
    let path = saved_path(data, f64::from(h.width), f64::from(h.height));
    (!path.is_empty()).then_some(path)
}

/// One record of a saved path: a selector, then 24 bytes.
const PATH_RECORD: usize = 26;

/// The saved path of path records `data`, scaled to an image `width` × `height` pixels. Knot
/// records come after their subpath's length record; one that comes before any, or past the knots
/// a length record counts, is left out, as is a last record cut short.
fn saved_path(data: &[u8], width: f64, height: f64) -> PathData {
    // A path point: a vertical, then a horizontal signed 8.24 fixed-point fraction of the image's
    // height and width.
    let fixed = |b: [u8; 4]| f64::from(i32::from_be_bytes(b)) / f64::from(1 << 24);
    let point = |[v0, v1, v2, v3, h0, h1, h2, h3]: [u8; 8]| Point::new(fixed([h0, h1, h2, h3]) * width, fixed([v0, v1, v2, v3]) * height);
    let mut subpaths: Vec<SubPath> = vec![];
    let mut all_pixels = false;
    // The knots the current subpath still takes.
    let mut left = 0usize;
    let (records, _) = data.as_chunks::<PATH_RECORD>();
    for &[s0, s1, ref body @ ..] in records {
        let [c0, c1, ..] = *body;
        let (points, _) = body.as_chunks::<8>();
        let [a, b, c] = [0, 1, 2].map(|i| points.get(i).copied().map_or(Point::ZERO, point));
        match u16::from_be_bytes([s0, s1]) {
            sel @ (0 | 3) => {
                left = usize::from(u16::from_be_bytes([c0, c1]));
                // Anchors are added as knots come: a count can claim more than follow.
                subpaths.push(SubPath::new(vec![], sel == 0));
            }
            sel @ (1 | 2 | 4 | 5) if left > 0 => {
                let Some(sp) = subpaths.last_mut() else { continue };
                left -= 1;
                let kind = if matches!(sel, 1 | 4) { AnchorKind::Smooth } else { AnchorKind::Corner };
                sp.anchors.push(Anchor { h_in: a, p: b, h_out: c, kind });
            }
            8 => all_pixels = [c0, c1] == [0, 1],
            _ => {}
        }
    }
    subpaths.retain(|s| !s.anchors.is_empty());
    if all_pixels && !subpaths.is_empty() {
        let all = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, width, height));
        subpaths.splice(0..0, all.subpaths);
    }
    PathData::new(subpaths)
}

/// Does the merged image keep its transparency in the first extra channel? Yes when the layer
/// count is negative: in the layer info, or for 16- and 32-bit documents (whose layer info is
/// empty) in their `Lr16` / `Lr32` block.
fn merged_alpha(layers: &[u8], psb: bool) -> bool {
    layer_info(layers, psb).is_some_and(|info| info.get(..2).is_some_and(|c| i16::from_be_bytes([c[0], c[1]]) < 0))
}

/// Do tagged blocks with this key give an 8-byte length in a PSB?
pub(super) fn long_key(key: &[u8; 4]) -> bool {
    matches!(key, b"Lr16" | b"Lr32" | b"Layr" | b"LMsk" | b"Mt16" | b"Mt32" | b"Mtrn" | b"Alph" | b"FMsk" | b"lnk2" | b"FEid" | b"FXid" | b"PxSD")
}

/// The layer info: the layer count, the layer records and their channels. It is the first part of
/// the layer and mask information, or for 16- and 32-bit documents (whose first part is empty) the
/// `Lr16` / `Lr32` block after it. `None` when the file has neither.
pub(super) fn layer_info(layers: &[u8], psb: bool) -> Option<&[u8]> {
    let mut r = Reader::new(layers);
    let info = r.section(psb).ok()?;
    if !info.is_empty() {
        return Some(info);
    }
    // The global layer mask info, then tagged blocks: signature, key, length (8 bytes for the
    // layer blocks of a PSB), data padded to 4 bytes.
    r.section(false).ok()?;
    while r.rest().len() >= 12 {
        r.take(4).ok()?;
        let key = r.array::<4>().ok()?;
        let data = r.section(psb && long_key(&key)).ok()?;
        if matches!(&key, b"Lr16" | b"Lr32" | b"Layr") {
            return Some(data);
        }
        r.take((4 - data.len() % 4) % 4).ok()?;
    }
    None
}

/// The most bytes one channel row may take (a PSB at 32 bits).
pub(super) fn row_bytes(width: usize, depth: u16) -> usize {
    if depth == 1 { width.div_ceil(8) } else { width * usize::from(depth / 8) }
}

/// Undo PackBits: `src` into `dst` (a row; a short or overlong run leaves the rest as it is).
pub(super) fn unpack_bits(mut src: &[u8], dst: &mut [u8]) {
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
        2 | 3 => inflate(r.rest(), wanted, method == 3, h.depth, row),
        m => Err(format!("the PSD file's image data uses an unknown compression ({m})")),
    }
}

/// `wanted` bytes of zipped image data, rows of `row` bytes; with `predicted`, each row stores the
/// difference from the sample before it. In 32-bit data a row stores the first bytes of its samples,
/// then their second bytes, and so on, and the differences run byte by byte along the whole row (as
/// psd-tools, MIT, reads it: <https://github.com/psd-tools/psd-tools>, `compression/__init__.py`).
fn inflate(src: &[u8], wanted: usize, predicted: bool, depth: u16, row: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(src)
        .take(wanted as u64)
        .read_to_end(&mut out)
        .map_err(|e| format!("the PSD file's image data can't be read: {e}"))?;
    if out.len() < wanted {
        return Err(TRUNCATED.into());
    }
    if predicted && row > 0 {
        match depth {
            8 => out.chunks_exact_mut(row).for_each(|r| (1..r.len()).for_each(|i| r[i] = r[i].wrapping_add(r[i - 1]))),
            16 => out.chunks_exact_mut(row).for_each(|r| {
                let mut prev = 0u16;
                for s in r.as_chunks_mut::<2>().0 {
                    prev = prev.wrapping_add(u16::from_be_bytes(*s));
                    *s = prev.to_be_bytes();
                }
            }),
            32 => {
                let w = row / 4;
                let mut planes = vec![0u8; row];
                for r in out.chunks_exact_mut(row) {
                    let mut prev = 0u8;
                    for (dst, &b) in planes.iter_mut().zip(r.iter()) {
                        prev = prev.wrapping_add(b);
                        *dst = prev;
                    }
                    for (x, s) in r.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        for (k, v) in s.iter_mut().enumerate() {
                            *v = planes.get(k * w + x).copied().unwrap_or(0);
                        }
                    }
                }
            }
            // 1-bit.
            _ => return Err("zipped 1-bit PSD image data with prediction can't be read".into()),
        }
    }
    Ok(out)
}

/// One channel of a layer: its compression method, then `rows` rows of `row` bytes (PackBits rows
/// after their byte counts, 4 bytes each in a PSB).
pub(super) fn layer_channel(data: &[u8], rows: usize, row: usize, psb: bool, depth: u16) -> Result<Vec<u8>, String> {
    let mut r = Reader::new(data);
    let method = r.u16()?;
    let wanted = rows.checked_mul(row).ok_or(TRUNCATED)?;
    match method {
        0 => r.take(wanted).map(<[u8]>::to_vec),
        1 => {
            let size = if psb { 4 } else { 2 };
            let counts = r.take(rows.checked_mul(size).ok_or(TRUNCATED)?)?;
            let mut out = vec![0; wanted];
            for (dst, n) in out.chunks_exact_mut(row.max(1)).zip(counts.chunks_exact(size)) {
                let n = n.iter().fold(0usize, |n, &b| n << 8 | usize::from(b));
                unpack_bits(r.take(n)?, dst);
            }
            Ok(out)
        }
        2 | 3 => inflate(r.rest(), wanted, method == 3, depth, row),
        m => Err(format!("the PSD file's layer data uses an unknown compression ({m})")),
    }
}

/// sRGB's transfer curve, for 32-bit documents, whose values are linear.
fn srgb_encode(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Sample `x` of a channel row, 0..1.
pub(super) fn sample(row: &[u8], x: usize, depth: u16, linear: bool) -> f32 {
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

pub(super) fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The sRGB color of a pixel whose color channels `v` gives (0..1, as stored), in `h`'s mode.
pub(super) fn to_rgb(h: &Header<'_>, v: impl Fn(usize) -> f32, cmyk: &impl Fn([f32; 4]) -> [f32; 3]) -> [f32; 3] {
    match h.mode {
        Mode::Bitmap | Mode::Gray => [v(0); 3],
        Mode::Indexed => {
            let i = usize::from(byte(v(0)));
            [0, 256, 512].map(|at| h.palette.get(at + i).map_or(0.0, |&c| f32::from(c) / 255.0))
        }
        Mode::Rgb => [v(0), v(1), v(2)],
        // Inks are stored as 1 − amount.
        Mode::Cmyk => cmyk([0, 1, 2, 3].map(|c| 1.0 - v(c))),
        Mode::Lab => lab_to_srgb(Lab { l: v(0) * 100.0, a: v(1) * 255.0 - 128.0, b: v(2) * 255.0 - 128.0 }),
    }
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
            let rgb = to_rgb(&h, v, &cmyk);
            let a = if alpha { v(colors) } else { 1.0 };
            // The merged image is stored on white: take the white back out.
            let straight = |c: f32| if a <= 0.0 { 0.0 } else { (c - (1.0 - a)) / a };
            let rgb = if alpha { rgb.map(straight) } else { rgb };
            px.0 = [byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(a)];
        }
    }
    Ok(out)
}
