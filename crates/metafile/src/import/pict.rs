//! QuickDraw pictures (PICT, versions 1 and 2), played on the [`Player`] by the opcodes and data
//! lengths of Apple's *Imaging With QuickDraw*, appendix A.
//!
//! A PICT file holds 512 bytes of application data before the picture; a picture on its own (as
//! a `PICT` resource holds it) reads too. A unit is a point, and the artboard is the picture's
//! frame; an extended version 2 header's source rectangle is mapped onto the frame.
//!
//! - Lines, rectangles, rounded rectangles, ovals, arcs and wedges, polygons and regions become
//!   paths. A framed shape is stroked inside its outline at the pen's size, and a line below and
//!   to the right of its points, where the pen draws; painted, filled and erased shapes are filled
//!   with the pen, fill or background pattern. A pattern of two colors is filled with their
//!   average.
//! - Text becomes point type in the font the picture names, read as Mac Roman.
//! - Bitmaps and pixel maps (1 to 8 bits with their color table, 16 and 32 bits, packed or not)
//!   become images, clipped to their mask region; QuickTime JPEG images become images.
//! - The clip region clips what follows it.
//!
//! What isn't read (inverted shapes, transfer modes other than a copy, other QuickTime formats)
//! comes back as one warning each. Apple's table gives every opcode's length, so opcodes that draw
//! nothing VectorCraft reads are skipped; a picture cut short keeps what came before.

use std::collections::HashMap;
use std::io::Cursor;

use vectorcraft_geom::{Affine, BezPath, FillRule, Point, Rect, Shape, Vec2, shapes};

use super::{BAD_IMAGE, FontObj, PS_GEOMETRIC, PenObj, Player, SRCCOPY, TOO_MANY_PIXELS, crop, round_rect};
use crate::dib::{self, Rgba};
use crate::{Imported, Kind};

/// What a file that holds no picture is refused with.
const NOT_PICT: &str = "not a PICT file";
/// The application data before the picture in a PICT file.
const FILE_HEADER: usize = 512;
/// Most opcodes one picture plays (a small file can't keep the reader busy for minutes).
const MAX_OPS: usize = 1 << 22;
/// Most rectangles one region breaks into.
const MAX_RECTS: usize = 1 << 16;
/// Steps the regions of one picture may take to read (each inversion point and band costs some).
const MAX_REGION_WORK: u64 = 1 << 28;
/// Most bytes the unpacked rows of one image may hold.
const MAX_ROW_BYTES: u64 = 1 << 28;
/// Most bytes the unpacked rows of one picture's images and patterns may hold together.
const MAX_FILE_ROW_BYTES: u64 = 1 << 30;
/// Most bytes a row may hold past what its pixels need.
const ROW_SLACK: usize = 64;
/// Most pixels, and bytes of unpacked rows, a pixel pattern may hold.
const MAX_PATTERN_PIXELS: usize = 1 << 16;
const MAX_PATTERN_ROW_BYTES: usize = 1 << 20;
/// Curves made for ovals and arcs are this close to the true curve, points.
const TOLERANCE: f64 = 0.01;
/// GDI text alignment on the baseline, which QuickDraw's text location is.
const TA_BASELINE: u32 = 0x18;
/// Pen style flags: square and flat ends, mitered joins.
const SQUARE: u32 = 0x100;
const FLAT: u32 = 0x200;
const MITER: u32 = 0x2000;

const CUT_SHORT: &str = "the picture is cut short or damaged: what came before was imported";
const TOO_MANY_OPS: &str = "the picture holds more opcodes than one file may: the rest was left out";
const INVERTED: &str = "inverted shapes were left out";
const PATTERNS: &str = "patterns are filled with the average of their colors";
const MODES: &str = "art drawn in transfer modes other than a copy is drawn as a copy";
const FRAMED_REGIONS: &str = "framed regions are outlined rectangle by rectangle";
const BIG_REGIONS: &str = "regions larger than one picture may hold were left out";
const CODECS: &str = "QuickTime images in formats other than JPEG were left out";
const NOTHING: &str = "the picture draws nothing VectorCraft reads";

/// Big-endian reads from a slice, never past its end.
struct Be<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Be<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(b)
    }
    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.bytes(N)?.try_into().ok()
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.bytes(n).map(drop)
    }
    fn u8(&mut self) -> Option<u8> {
        self.array().map(|[b]: [u8; 1]| b)
    }
    fn u16(&mut self) -> Option<u16> {
        self.array().map(u16::from_be_bytes)
    }
    fn i16(&mut self) -> Option<i16> {
        self.array().map(i16::from_be_bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_be_bytes)
    }
    fn i32(&mut self) -> Option<i32> {
        self.array().map(i32::from_be_bytes)
    }
    /// A QuickDraw point: vertical, then horizontal.
    fn point(&mut self) -> Option<Point> {
        let v = self.i16()?;
        let h = self.i16()?;
        Some(Point::new(h.into(), v.into()))
    }
    /// A size stored as a point (a pen or an oval: height, then width).
    fn size(&mut self) -> Option<Vec2> {
        self.point().map(Point::to_vec2)
    }
    /// A QuickDraw rectangle: top, left, bottom, right.
    fn rect(&mut self) -> Option<Rect> {
        let (top, left, bottom, right) = (self.i16()?, self.i16()?, self.i16()?, self.i16()?);
        Some(Rect::new(left.into(), top.into(), right.into(), bottom.into()))
    }
    /// An `RGBColor` (16 bits a component), 8 bits a component.
    fn rgb(&mut self) -> Option<[u8; 3]> {
        let [r, g, b] = [self.u16()?, self.u16()?, self.u16()?];
        Some([r, g, b].map(|c| (c >> 8) as u8))
    }
}

/// Where a picture is, its frame and whether it is version 2.
struct Header {
    frame: Rect,
    v2: bool,
    /// Where its first opcode starts.
    ops: usize,
}

/// The picture whose `picSize` is at `at`, if a version opcode follows its frame.
fn header(bytes: &[u8], at: usize) -> Option<Header> {
    let mut r = Be::new(bytes, at.checked_add(2)?);
    let frame = r.rect()?;
    let (v2, ops) = match r.array::<2>()? {
        [0x11, 0x01] => (false, r.pos),
        [0x00, 0x11] if r.array::<4>()? == [0x02, 0xFF, 0x0C, 0x00] => (true, at + 10),
        _ => return None,
    };
    (frame.width() > 0.0 && frame.height() > 0.0).then_some(Header { frame, v2, ops })
}

/// The picture of a file (after its application data), else the picture `bytes` start with.
fn find(bytes: &[u8]) -> Option<Header> {
    header(bytes, FILE_HEADER).or_else(|| header(bytes, 0))
}

/// Do `bytes` hold a version 2 picture, after a file's application data or on their own? The
/// version 1 signature is two bytes, too short to tell a file by.
pub(crate) fn is_pict(bytes: &[u8]) -> bool {
    [FILE_HEADER, 0].into_iter().any(|at| header(bytes, at).is_some_and(|h| h.v2))
}

/// A pattern: eight rows of eight bits (set bits in the foreground color, the others in the
/// background color), or one color.
#[derive(Clone, Copy)]
enum Pat {
    Bits([u8; 8]),
    Rgb([u8; 3]),
}

/// What a shape opcode does with its shape.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Verb {
    Frame,
    Paint,
    Erase,
    Invert,
    Fill,
}

/// The verb of a shape opcode (its low three bits; 5 to 7 are reserved).
fn verb(op: u16) -> Option<Verb> {
    match op & 7 {
        0 => Some(Verb::Frame),
        1 => Some(Verb::Paint),
        2 => Some(Verb::Erase),
        3 => Some(Verb::Invert),
        4 => Some(Verb::Fill),
        _ => None,
    }
}

/// An old-style QuickDraw color (`blackColor` 33 … `blueColor` 409) as RGB.
fn old_color(c: u32) -> [u8; 3] {
    match c {
        30 => [255; 3],
        69 => [255, 255, 0],
        137 => [255, 0, 255],
        205 => [255, 0, 0],
        273 => [0, 255, 255],
        341 => [0, 255, 0],
        409 => [0, 0, 255],
        _ => [0; 3],
    }
}

/// `a` and `b` mixed: `t` of `a`.
fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let m = |x: u8, y: u8| (f64::from(x) * t + f64::from(y) * (1.0 - t)).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

/// Byte `b` of Mac Roman.
fn mac_roman(b: u8) -> char {
    const HIGH: [char; 128] = [
        'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', //
        'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', //
        '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', //
        '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', //
        '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{A0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', //
        '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ', //
        '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô', //
        '\u{F8FF}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
    ];
    match b {
        0x80.. => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or(char::REPLACEMENT_CHARACTER),
        _ => char::from(b),
    }
}

/// The family name of a classic Macintosh font number.
fn font_family(id: u16) -> &'static str {
    match id {
        0 => "Chicago",
        1 | 3 => "Geneva",
        2 => "New York",
        4 => "Monaco",
        20 => "Times",
        21 => "Helvetica",
        22 => "Courier",
        23 => "Symbol",
        _ => "",
    }
}

/// The arc of the oval in `r` from `start` degrees clockwise from 12 o'clock, over `sweep`
/// degrees (clockwise when positive). Angles are measured on the rectangle, so 45° runs through
/// its top-right corner. A wedge closes through the center.
fn arc(r: Rect, start: i16, sweep: i16, wedge: bool) -> BezPath {
    let c = r.center();
    let radii = Vec2::new(r.width() / 2.0, r.height() / 2.0);
    let mut bp = BezPath::new();
    if sweep == 0 {
        return bp;
    }
    let a0 = (f64::from(start) - 90.0).to_radians();
    let sweep = f64::from(sweep).clamp(-360.0, 360.0).to_radians();
    let arc = kurbo::Arc { center: c, radii, start_angle: a0, sweep_angle: sweep, x_rotation: 0.0 };
    if wedge {
        bp.move_to(c);
        bp.line_to(c + Vec2::new(radii.x * a0.cos(), radii.y * a0.sin()));
        arc.append_iter(TOLERANCE).for_each(|el| bp.push(el));
        bp.close_path();
    } else {
        kurbo::Shape::path_elements(&arc, TOLERANCE).for_each(|el| bp.push(el));
    }
    bp
}

/// The outlines of `rects`, as one path.
fn rects_path(rects: &[Rect]) -> BezPath {
    let mut bp = BezPath::new();
    for r in rects {
        bp.extend(shapes::rectangle(*r).to_bezpath());
    }
    bp
}

/// Take `n` steps from `work`; false when too few are left.
fn spend(work: &mut u64, n: usize) -> bool {
    match work.checked_sub(n as u64) {
        Some(left) => {
            *work = left;
            true
        }
        None => false,
    }
}

/// A region (its size, bounding box and inversion points), read at `r`, as rectangles: `None`
/// when the data runs past the end; the flag is false when the region is larger than `work` or
/// [`MAX_RECTS`] allows, and then no rectangles are given.
fn region(r: &mut Be, work: &mut u64) -> Option<(Vec<Rect>, bool)> {
    let size = usize::from(r.u16()?);
    let mut b = Be::new(r.bytes(size.checked_sub(2)?)?, 0);
    let bbox = b.rect()?;
    if size <= 10 {
        let whole = bbox.width() > 0.0 && bbox.height() > 0.0;
        return Some((if whole { vec![bbox] } else { vec![] }, true));
    }
    Some(bands(&mut b, work).map_or((vec![], false), |rects| (rects, true)))
}

/// The rectangles of a region's scanlines: each lists the x coordinates where being inside the
/// region changes from the scanline before. Bands of the same spans one after another are one
/// rectangle.
fn bands(b: &mut Be, work: &mut u64) -> Option<Vec<Rect>> {
    let mut out: Vec<Rect> = vec![];
    let mut active: Vec<i16> = vec![];
    let mut last_band = 0;
    let mut prev_y = 0.0;
    while let Some(y) = b.i16().filter(|y| *y != 0x7FFF) {
        let y = f64::from(y);
        let spans: Vec<(f64, f64)> = active.as_chunks::<2>().0.iter().map(|[a, b]| (f64::from(*a), f64::from(*b))).collect();
        if !spans.is_empty() && y > prev_y {
            if !spend(work, spans.len() + 1) {
                return None;
            }
            let prev = out.get_mut(last_band..).unwrap_or_default();
            let same = prev.len() == spans.len() && prev.iter().zip(&spans).all(|(r, (x0, x1))| r.x0 == *x0 && r.x1 == *x1 && r.y1 == prev_y);
            if same {
                prev.iter_mut().for_each(|r| r.y1 = y);
            } else {
                last_band = out.len();
                out.extend(spans.iter().map(|&(x0, x1)| Rect::new(x0, prev_y, x1, y)));
                if out.len() > MAX_RECTS {
                    return None;
                }
            }
        }
        prev_y = y;
        while let Some(x) = b.i16().filter(|x| *x != 0x7FFF) {
            if !spend(work, active.len() + 1) {
                return None;
            }
            match active.binary_search(&x) {
                Ok(i) => {
                    active.remove(i);
                }
                Err(i) => active.insert(i, x),
            }
        }
    }
    Some(out)
}

/// Skip the data of opcode `op`, by the length Apple's table gives it.
fn skip(op: u16, r: &mut Be) -> Option<()> {
    let n = match op {
        0x00 | 0x17..=0x19 | 0x1C | 0x1E | 0xB0..=0xCF | 0x8000..=0x80FF => 0,
        0x04 => 1,
        0x03 | 0x05 | 0x08 | 0x0D | 0x11 | 0x15 | 0x16 | 0xA0 => 2,
        0x06 | 0x07 | 0x0B | 0x0C | 0x0E | 0x0F => 4,
        0x1A | 0x1B | 0x1D | 0x1F => 6,
        0x02 | 0x09 | 0x0A | 0x10 => 8,
        // Reserved opcodes with a 16-bit length, and the font name.
        0x24..=0x27 | 0x2C..=0x2F | 0x92..=0x97 | 0x9C..=0x9F | 0xA2..=0xAF => usize::from(r.u16()?),
        // Reserved for Apple: two bytes for each unit of the high byte (the 24-byte header 0x0C00).
        0x0100..=0x7FFF => usize::from(op >> 8) * 2,
        // 0xD0–0xFE and 0x8100–0xFFFF (QuickTime images among them): a 32-bit length.
        _ => usize::try_from(r.u32()?).ok()?,
    };
    r.skip(n)
}

/// How a bitmap's or pixel map's pixels are stored.
struct PixMap {
    row_bytes: usize,
    bounds: Rect,
    /// Bits a pixel.
    size: u16,
    pack: u16,
    cmps: u16,
    /// Indexed pixels' colors; a bitmap has none (its set bits are the foreground color).
    palette: Option<Vec<[u8; 3]>>,
    /// Rows of 8 bytes or more may be packed with PackBits, as the packing says.
    packed: bool,
}

impl PixMap {
    /// From `rowBytes` on: a pixel map when `rowBytes` has its high bit set or `pixmap` says so,
    /// else a bitmap; a pixel map's color table follows unless `direct`.
    fn read(r: &mut Be, pixmap: bool, direct: bool) -> Option<Self> {
        let raw = r.u16()?;
        let mut pm = PixMap { row_bytes: usize::from(raw & 0x3FFF), bounds: r.rect()?, size: 1, pack: 0, cmps: 1, palette: None, packed: true };
        if pixmap || raw & 0x8000 != 0 {
            r.skip(2)?;
            pm.pack = r.u16()?;
            r.skip(14)?;
            pm.size = r.u16()?;
            pm.cmps = r.u16()?;
            r.skip(14)?;
            if !direct {
                pm.palette = Some(color_table(r)?);
            }
        }
        Some(pm)
    }

    /// The width and height of its pixels.
    fn dims(&self) -> Option<(usize, usize)> {
        let (w, h) = (self.bounds.width(), self.bounds.height());
        (w > 0.0 && h > 0.0).then_some((w as usize, h as usize))
    }

    /// The packing: 0 is 4 for 32-bit pixels and 3 for 16-bit ones.
    fn packing(&self) -> u16 {
        match (self.size, self.pack) {
            (32, 0) => 4,
            (16, 0) => 3,
            (_, p) => p,
        }
    }

    /// Rows are stored as they are (no byte count, no PackBits).
    fn raw_rows(&self) -> bool {
        !self.packed || self.row_bytes < 8 || self.packing() == 1 || (self.size == 32 && self.packing() == 2)
    }

    /// Bytes of an unpacked row: a component plane each for packing 4, three bytes a pixel for
    /// packing 2, else `rowBytes`.
    fn row_len(&self, w: usize) -> usize {
        match self.packing() {
            4 if self.size == 32 && self.row_bytes >= 8 => w * usize::from(self.cmps),
            2 if self.size == 32 => w * 3,
            _ => self.row_bytes,
        }
    }

    /// Whether its pixels can be read: a depth QuickDraw has, and rows long enough for them and at
    /// most [`ROW_SLACK`] bytes longer.
    fn decodable(&self, w: usize) -> bool {
        let need = match self.size {
            1 | 2 | 4 | 8 => (w * usize::from(self.size)).div_ceil(8),
            16 => 2 * w,
            32 if self.packing() == 4 && self.row_bytes >= 8 => {
                if !matches!(self.cmps, 3 | 4) {
                    return false;
                }
                w * usize::from(self.cmps)
            }
            32 if self.packing() == 2 => 3 * w,
            32 => 4 * w,
            _ => return false,
        };
        (need..=need + ROW_SLACK).contains(&self.row_len(w))
    }

    /// Read `h` rows of pixel data, unpacked into one buffer when `keep`, else only stepped over.
    /// `None` when the data runs past the end; the flag is false when a row didn't unpack to its
    /// length.
    fn rows(&self, r: &mut Be, w: usize, h: usize, keep: bool) -> Option<(Vec<u8>, bool)> {
        let len = self.row_len(w);
        let unit = if self.packing() == 3 { 2 } else { 1 };
        let mut out = vec![];
        let mut ok = true;
        let mut keep = keep;
        for _ in 0..h {
            if self.raw_rows() {
                let row = r.bytes(len)?;
                if keep {
                    out.extend_from_slice(row);
                }
                continue;
            }
            let n = if self.row_bytes > 250 { usize::from(r.u16()?) } else { usize::from(r.u8()?) };
            let packed = r.bytes(n)?;
            // After a row that doesn't unpack, the rest are only stepped over.
            if keep && !unpack(packed, len, unit, &mut out) {
                ok = false;
                keep = false;
                out = vec![];
            }
        }
        Some((out, ok))
    }

    /// The color of pixel `x` of `row` (the foreground and background colors for a bitmap's bits).
    fn pixel(&self, row: &[u8], x: usize, w: usize, fg: [u8; 3], bg: [u8; 3]) -> Option<[u8; 3]> {
        let at = |i: usize| row.get(i).copied();
        match self.size {
            1 | 2 | 4 | 8 => {
                let bits = usize::from(self.size);
                let shift = 8 - bits - x * bits % 8;
                let index = usize::from((at(x * bits / 8)? >> shift) & ((1u16 << bits) - 1) as u8);
                match &self.palette {
                    Some(p) => p.get(index).copied(),
                    None => Some(if index == 0 { bg } else { fg }),
                }
            }
            16 => {
                let v = u16::from_be_bytes([at(2 * x)?, at(2 * x + 1)?]);
                let c = |s: u16| (((v >> s) & 31) * 255 / 31) as u8;
                Some([c(10), c(5), c(0)])
            }
            _ if self.packing() == 4 && self.row_bytes >= 8 => {
                // Component planes, alpha first when there are four (QuickDraw doesn't draw it).
                let first = usize::from(self.cmps) - 3;
                Some([at(first * w + x)?, at((first + 1) * w + x)?, at((first + 2) * w + x)?])
            }
            _ if self.packing() == 2 => Some([at(3 * x)?, at(3 * x + 1)?, at(3 * x + 2)?]),
            _ => Some([at(4 * x + 1)?, at(4 * x + 2)?, at(4 * x + 3)?]),
        }
    }

    /// Unpacked rows as opaque pixels.
    fn to_rgba(&self, data: &[u8], w: usize, h: usize, fg: [u8; 3], bg: [u8; 3]) -> Option<Rgba> {
        let len = self.row_len(w);
        let mut pixels = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let row = data.get(y * len..(y + 1) * len)?;
            for x in 0..w {
                pixels.extend(self.pixel(row, x, w, fg, bg)?);
                pixels.push(255);
            }
        }
        Some(Rgba { width: u32::try_from(w).ok()?, height: u32::try_from(h).ok()?, pixels })
    }
}

/// A `ColorTable`: indexed pixels' colors (entry by entry for a device table, else by each
/// entry's value).
fn color_table(r: &mut Be) -> Option<Vec<[u8; 3]>> {
    r.skip(4)?;
    let flags = r.u16()?;
    let n = usize::from(r.u16()?) + 1;
    let mut palette = vec![[0u8; 3]; 256];
    for i in 0..n {
        let value = usize::from(r.u16()?);
        let c = r.rgb()?;
        if let Some(e) = palette.get_mut(if flags & 0x8000 != 0 { i } else { value }) {
            *e = c;
        }
    }
    Some(palette)
}

/// Unpack PackBits `data` (runs of `unit`-byte elements) onto `out`, `len` bytes; false when the
/// data is damaged or short (the row is padded with zeros).
fn unpack(data: &[u8], len: usize, unit: usize, out: &mut Vec<u8>) -> bool {
    let start = out.len();
    let mut i = 0;
    let mut ok = true;
    while out.len() - start < len {
        let Some(&n) = data.get(i) else {
            ok = false;
            break;
        };
        i += 1;
        let n = n as i8;
        if n >= 0 {
            let k = (n as usize + 1) * unit;
            let Some(lit) = data.get(i..i + k) else {
                ok = false;
                break;
            };
            out.extend_from_slice(lit);
            i += k;
        } else if n != -128 {
            let Some(v) = data.get(i..i + unit) else {
                ok = false;
                break;
            };
            for _ in 0..(1 - isize::from(n)) {
                out.extend_from_slice(v);
            }
            i += unit;
        }
    }
    out.resize(start + len, 0);
    ok
}

/// The picture being played: QuickDraw's drawing state, and the player that makes the objects.
struct Pict {
    p: Player,
    frame: Rect,
    /// Picture space → document.
    base: Affine,
    origin: Vec2,
    fg: [u8; 3],
    bg: [u8; 3],
    pen: Vec2,
    pen_mode: u16,
    pen_pat: Pat,
    fill_pat: Pat,
    back_pat: Pat,
    oval: Vec2,
    pen_loc: Point,
    font: u16,
    face: u8,
    text_size: i16,
    text_loc: Point,
    /// Font numbers the picture names.
    names: HashMap<u16, String>,
    /// The last rectangle of the rectangle, rounded rectangle, oval and arc opcodes.
    last: [Rect; 4],
    region_work: u64,
    /// Bytes of unpacked rows the picture's images and patterns may still take.
    row_bytes_left: u64,
}

impl Pict {
    fn new(frame: Rect) -> Self {
        let base = Affine::translate(-frame.origin().to_vec2());
        let artboard = Rect::new(0.0, 0.0, frame.width(), frame.height());
        // A picture's coordinates map straight onto the document, as a WMF's do.
        let p = Player::new(Kind::Wmf, base, artboard);
        Self {
            p,
            frame,
            base,
            origin: Vec2::ZERO,
            fg: [0; 3],
            bg: [255; 3],
            pen: Vec2::new(1.0, 1.0),
            pen_mode: 8,
            pen_pat: Pat::Bits([0xFF; 8]),
            fill_pat: Pat::Bits([0xFF; 8]),
            back_pat: Pat::Bits([0; 8]),
            oval: Vec2::ZERO,
            pen_loc: Point::ZERO,
            font: 0,
            face: 0,
            text_size: 0,
            text_loc: Point::ZERO,
            names: HashMap::new(),
            last: [Rect::ZERO; 4],
            region_work: MAX_REGION_WORK,
            row_bytes_left: MAX_FILE_ROW_BYTES,
        }
    }

    /// Charge the `w` × `h` pixels of `pm`, and the bytes of its unpacked rows, to the picture's
    /// budgets: false, and nothing charged, when either is spent.
    fn charge(&mut self, pm: &PixMap, w: usize, h: usize) -> bool {
        let bytes = (pm.row_len(w) as u64).saturating_mul(h as u64);
        let Some(rows_left) = self.row_bytes_left.checked_sub(bytes) else { return false };
        let (Ok(wide), Ok(high)) = (u32::try_from(w), u32::try_from(h)) else { return false };
        if dib::charge(wide, high, &mut self.p.pixels_left).is_err() {
            return false;
        }
        self.row_bytes_left = rows_left;
        true
    }

    /// Play opcode `op`, its data at `r`; `None` when the data runs past the end.
    fn op(&mut self, op: u16, r: &mut Be) -> Option<()> {
        if !matches!(op, 0x20..=0x23) {
            self.p.flush();
        }
        match op {
            0x01 => self.clip(r),
            0x0C => self.move_origin(r),
            0x20..=0x23 => self.line(op, r),
            0x28..=0x2B => self.text(op, r),
            0x2C => self.font_name(r),
            0x30..=0x6F => self.shape(op, r),
            0x70..=0x77 => self.poly(verb(op), r),
            0x80..=0x87 => self.region_shape(verb(op), r),
            // The same polygon and region: Apple's table gives them no data and no drawing.
            0x78..=0x7F | 0x88..=0x8F => Some(()),
            0x90 | 0x91 | 0x98..=0x9B => self.bits(op, r),
            0xA1 => {
                r.skip(2)?;
                let n = usize::from(r.u16()?);
                r.skip(n)
            }
            0x0C00 => self.header(r),
            0x8200 => self.quicktime(r),
            _ => self.state(op, r),
        }
    }

    /// The opcodes that set the drawing state; the others are skipped.
    fn state(&mut self, op: u16, r: &mut Be) -> Option<()> {
        match op {
            0x02 => self.back_pat = Pat::Bits(r.array()?),
            0x03 => self.font = r.u16()?,
            0x04 => self.face = r.u8()?,
            0x07 => self.pen = r.size()?,
            0x08 => self.pen_mode = r.u16()?,
            0x09 => self.pen_pat = Pat::Bits(r.array()?),
            0x0A => self.fill_pat = Pat::Bits(r.array()?),
            0x0B => self.oval = r.size()?,
            0x0D => self.text_size = r.i16()?,
            0x0E => self.fg = old_color(r.u32()?),
            0x0F => self.bg = old_color(r.u32()?),
            0x12 => self.back_pat = self.pix_pat(r)?,
            0x13 => self.pen_pat = self.pix_pat(r)?,
            0x14 => self.fill_pat = self.pix_pat(r)?,
            0x1A => self.fg = r.rgb()?,
            0x1B => self.bg = r.rgb()?,
            _ => return skip(op, r),
        }
        Some(())
    }

    /// The version 2 header: an extended one's source rectangle maps onto the frame.
    fn header(&mut self, r: &mut Be) -> Option<()> {
        let mut b = Be::new(r.bytes(24)?, 0);
        if b.i16() == Some(-2)
            && let Some(src) = b.skip(10).and_then(|()| b.rect())
            && src.width() > 0.0
            && src.height() > 0.0
        {
            let k = Vec2::new(self.frame.width() / src.width(), self.frame.height() / src.height());
            self.base = Affine::scale_non_uniform(k.x, k.y) * Affine::translate(-src.origin().to_vec2());
            self.p.dev_to_doc = self.base * Affine::translate(-self.origin);
        }
        Some(())
    }

    /// The origin moves by (dh, dv): a point drawn after it is that much nearer the top left.
    fn move_origin(&mut self, r: &mut Be) -> Option<()> {
        let (dh, dv) = (r.i16()?, r.i16()?);
        self.origin += Vec2::new(dh.into(), dv.into());
        self.p.dev_to_doc = self.base * Affine::translate(-self.origin);
        Some(())
    }

    /// A pixel pattern: its color (the average of a color pattern's pixels), else its bitmap.
    fn pix_pat(&mut self, r: &mut Be) -> Option<Pat> {
        let kind = r.u16()?;
        let bits: [u8; 8] = r.array()?;
        match kind {
            1 => {
                let pm = PixMap::read(r, true, false)?;
                let (w, h) = pm.dims()?;
                let small = w * h <= MAX_PATTERN_PIXELS && pm.row_len(w) * h <= MAX_PATTERN_ROW_BYTES;
                let keep = small && pm.decodable(w) && self.charge(&pm, w, h);
                let (data, _) = pm.rows(r, w, h, keep)?;
                let avg = keep.then(|| pm.to_rgba(&data, w, h, self.fg, self.bg)).flatten().map(|img| self.average(&img));
                Some(avg.map_or(Pat::Bits(bits), Pat::Rgb))
            }
            2 => Some(Pat::Rgb(r.rgb()?)),
            _ => Some(Pat::Bits(bits)),
        }
    }

    /// The average color of `img`, with a warning when its pixels differ.
    fn average(&mut self, img: &Rgba) -> [u8; 3] {
        let px = img.pixels.as_chunks::<4>().0;
        if px.windows(2).any(|w| w[0] != w[1]) {
            self.p.warn(PATTERNS);
        }
        let n = px.len().max(1) as u64;
        let sum = |i: usize| (px.iter().map(|p| u64::from(p[i])).sum::<u64>() / n) as u8;
        [sum(0), sum(1), sum(2)]
    }

    /// The color a pattern fills with: its own, or its two colors mixed as its bits are.
    fn pat_color(&mut self, pat: Pat) -> [u8; 3] {
        match pat {
            Pat::Rgb(c) => c,
            Pat::Bits(b) => {
                let ones: u32 = b.iter().map(|x| x.count_ones()).sum();
                if ones != 0 && ones != 64 {
                    self.p.warn(PATTERNS);
                }
                mix(self.fg, self.bg, f64::from(ones) / 64.0)
            }
        }
    }

    /// The pen's color (a warning when its mode isn't a copy).
    fn pen_color(&mut self) -> [u8; 3] {
        if !matches!(self.pen_mode, 0 | 8) {
            self.p.warn(MODES);
        }
        self.pat_color(self.pen_pat)
    }

    /// The color painting, erasing or filling an area takes; `None` for the other verbs.
    fn area_color(&mut self, verb: Verb) -> Option<[u8; 3]> {
        match verb {
            Verb::Paint => Some(self.pen_color()),
            Verb::Erase => Some(self.pat_color(self.back_pat)),
            Verb::Fill => Some(self.pat_color(self.fill_pat)),
            Verb::Frame | Verb::Invert => None,
        }
    }

    fn has_pen(&self) -> bool {
        self.pen.x > 0.0 && self.pen.y > 0.0
    }

    /// Fill `bp` (picture space) with `color`.
    fn fill(&mut self, mut bp: BezPath, color: [u8; 3], rule: FillRule) {
        bp.apply_affine(self.p.xf());
        self.p.dc.brush = color;
        self.p.dc.fill_rule = rule;
        self.p.draw(bp, true, false);
    }

    /// The pen as a stroke of `color` with `ends`, as wide as the pen's average side.
    fn set_pen(&mut self, color: [u8; 3], ends: u32) {
        let width = (self.pen.x + self.pen.y) / 2.0;
        self.p.dc.pen = PenObj { style: PS_GEOMETRIC | ends | MITER, width, color, entries: vec![] };
    }

    /// Stroke `bp` (picture space) with the pen in `color`.
    fn stroke(&mut self, mut bp: BezPath, color: [u8; 3], ends: u32) {
        self.set_pen(color, ends);
        bp.apply_affine(self.p.xf());
        self.p.draw(bp, false, true);
    }

    /// Draw a framed shape: its outline (from `outline` of a rectangle and an oval size) stroked
    /// inside `r`, or filled when the pen is as large as the shape.
    fn frame(&mut self, r: Rect, outline: impl Fn(Rect, Vec2) -> BezPath, ends: u32) {
        if !self.has_pen() {
            return;
        }
        let color = self.pen_color();
        let half = self.pen / 2.0;
        let inner = Rect::new(r.x0 + half.x, r.y0 + half.y, r.x1 - half.x, r.y1 - half.y);
        if inner.width() <= 0.0 || inner.height() <= 0.0 {
            return self.fill(outline(r, self.oval), color, FillRule::NonZero);
        }
        self.stroke(outline(inner, self.oval - self.pen), color, ends);
    }

    /// The rectangle, rounded rectangle, oval and arc opcodes.
    fn shape(&mut self, op: u16, r: &mut Be) -> Option<()> {
        let family = op >> 4;
        let slot = usize::from(family - 3);
        let rect = if op & 8 != 0 { self.last.get(slot).copied()? } else { r.rect()? };
        let angles = if family == 6 { Some((r.i16()?, r.i16()?)) } else { None };
        let Some(verb) = verb(op) else { return Some(()) };
        if let Some(l) = self.last.get_mut(slot) {
            *l = rect;
        }
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return Some(());
        }
        let outline = move |r: Rect, oval: Vec2, wedge: bool| match family {
            3 => shapes::rectangle(r).to_bezpath(),
            4 => round_rect(r, oval.x, oval.y),
            5 => kurbo::Ellipse::from_rect(r).to_path(TOLERANCE),
            _ => angles.map_or_else(BezPath::new, |(start, sweep)| arc(r, start, sweep, wedge)),
        };
        match verb {
            Verb::Frame => self.frame(rect, |r, oval| outline(r, oval, false), if family == 6 { FLAT } else { SQUARE }),
            Verb::Invert => self.p.warn(INVERTED),
            _ => {
                if let Some(c) = self.area_color(verb) {
                    self.fill(outline(rect, self.oval, true), c, FillRule::NonZero);
                }
            }
        }
        Some(())
    }

    /// A polygon opcode: framed along its points, else filled.
    fn poly(&mut self, verb: Option<Verb>, r: &mut Be) -> Option<()> {
        let size = usize::from(r.u16()?);
        let mut b = Be::new(r.bytes(size.checked_sub(2)?)?, 8);
        let Some(verb) = verb else { return Some(()) };
        let mut pts = vec![];
        while let Some(p) = b.point() {
            pts.push(p);
        }
        let Some((first, rest)) = pts.split_first() else { return Some(()) };
        let closed = pts.len() > 2 && pts.first() == pts.last();
        match verb {
            Verb::Frame if self.has_pen() => {
                let half = self.pen / 2.0;
                let mut bp = BezPath::new();
                bp.move_to(*first + half);
                rest.iter().for_each(|p| bp.line_to(*p + half));
                if closed {
                    bp.close_path();
                }
                let color = self.pen_color();
                self.stroke(bp, color, SQUARE);
            }
            Verb::Frame => {}
            Verb::Invert => self.p.warn(INVERTED),
            _ => {
                let mut bp = BezPath::new();
                bp.move_to(*first);
                rest.iter().for_each(|p| bp.line_to(*p));
                bp.close_path();
                if let Some(c) = self.area_color(verb) {
                    self.fill(bp, c, FillRule::EvenOdd);
                }
            }
        }
        Some(())
    }

    /// A region's rectangles, with a warning when it was too large to read.
    fn read_region(&mut self, r: &mut Be) -> Option<Vec<Rect>> {
        let (rects, whole) = region(r, &mut self.region_work)?;
        if !whole {
            self.p.warn(BIG_REGIONS);
        }
        Some(rects)
    }

    /// A region opcode: each rectangle framed, else the region filled.
    fn region_shape(&mut self, verb: Option<Verb>, r: &mut Be) -> Option<()> {
        let rects = self.read_region(r)?;
        let Some(verb) = verb.filter(|_| !rects.is_empty()) else { return Some(()) };
        match verb {
            Verb::Frame if self.has_pen() => {
                if rects.len() > 1 {
                    self.p.warn(FRAMED_REGIONS);
                }
                let half = self.pen / 2.0;
                let inner: Vec<Rect> = rects.iter().map(|r| Rect::new(r.x0 + half.x, r.y0 + half.y, r.x1 - half.x, r.y1 - half.y)).collect();
                let color = self.pen_color();
                self.stroke(rects_path(&inner), color, SQUARE);
            }
            Verb::Frame => {}
            Verb::Invert => self.p.warn(INVERTED),
            _ => {
                if let Some(c) = self.area_color(verb) {
                    self.fill(rects_path(&rects), c, FillRule::NonZero);
                }
            }
        }
        Some(())
    }

    /// The clip region: it replaces the clip. An empty region clips away everything after it, as
    /// a clip of no area at the frame's corner; one too large to read leaves the clip as it was.
    fn clip(&mut self, r: &mut Be) -> Option<()> {
        let (mut rects, whole) = region(r, &mut self.region_work)?;
        if !whole {
            self.p.warn(BIG_REGIONS);
            return Some(());
        }
        if rects.is_empty() {
            let corner = Point::new(self.frame.x0, self.frame.y0);
            rects.push(Rect::from_points(corner, corner));
        }
        let mut bp = rects_path(&rects);
        bp.apply_affine(self.p.xf());
        self.p.push_clip(bp, FillRule::NonZero, true);
        Some(())
    }

    /// The line opcodes: the pen drawn from point to point, hanging below and to the right of
    /// the line; lines one after another join into one path.
    fn line(&mut self, op: u16, r: &mut Be) -> Option<()> {
        let delta = |r: &mut Be| Some(Vec2::new(f64::from(r.u8()? as i8), f64::from(r.u8()? as i8)));
        let (from, to) = match op {
            0x20 => (r.point()?, r.point()?),
            0x21 => (self.pen_loc, r.point()?),
            0x22 => {
                let a = r.point()?;
                (a, a + delta(r)?)
            }
            _ => (self.pen_loc, self.pen_loc + delta(r)?),
        };
        self.pen_loc = to;
        if !self.has_pen() {
            return Some(());
        }
        let color = self.pen_color();
        if from == to {
            // A dot: the pen's rectangle.
            self.p.flush();
            self.fill(shapes::rectangle(Rect::from_points(from, from + self.pen)).to_bezpath(), color, FillRule::NonZero);
            return Some(());
        }
        let pen = PenObj { style: PS_GEOMETRIC | SQUARE | MITER, width: (self.pen.x + self.pen.y) / 2.0, color, entries: vec![] };
        if self.p.dc.pen != pen {
            self.p.flush();
            self.p.dc.pen = pen;
        }
        let half = self.pen / 2.0;
        let xf = self.p.xf();
        let (a, b) = (xf * (from + half), xf * (to + half));
        if self.p.pending.elements().last().and_then(|e| e.end_point()) != Some(a) {
            self.p.pending.move_to(a);
        }
        self.p.pending.line_to(b);
        Some(())
    }

    /// The text opcodes: a string at the text location, moved to or by the opcode's data.
    fn text(&mut self, op: u16, r: &mut Be) -> Option<()> {
        match op {
            0x28 => self.text_loc = r.point()?,
            0x29 => self.text_loc.x += f64::from(r.u8()?),
            0x2A => self.text_loc.y += f64::from(r.u8()?),
            _ => {
                self.text_loc.x += f64::from(r.u8()?);
                self.text_loc.y += f64::from(r.u8()?);
            }
        }
        let n = usize::from(r.u8()?);
        let s: String = r.bytes(n)?.iter().map(|&b| mac_roman(b)).filter(|c| !c.is_control()).collect();
        let size = if self.text_size > 0 { self.text_size } else { 12 };
        let face = self.names.get(&self.font).cloned().unwrap_or_else(|| font_family(self.font).to_string());
        self.p.dc.font = FontObj {
            height: -f64::from(size),
            escapement: 0.0,
            weight: if self.face & 1 != 0 { 700 } else { 400 },
            italic: self.face & 2 != 0,
            face,
        };
        self.p.dc.text_color = self.fg;
        self.p.dc.text_align = TA_BASELINE;
        self.p.text(self.text_loc, &s);
        Some(())
    }

    /// A font's name for its number.
    fn font_name(&mut self, r: &mut Be) -> Option<()> {
        let n = usize::from(r.u16()?);
        let mut b = Be::new(r.bytes(n)?, 0);
        if let Some(id) = b.u16()
            && let Some(len) = b.u8()
            && let Some(name) = b.bytes(usize::from(len))
        {
            let name: String = name.iter().map(|&c| mac_roman(c)).collect();
            self.names.insert(id, name.trim().to_string());
        }
        Some(())
    }

    /// A bitmap or pixel map copied into a rectangle (`Rgn` opcodes: clipped to a mask region).
    fn bits(&mut self, op: u16, r: &mut Be) -> Option<()> {
        let direct = matches!(op, 0x9A | 0x9B);
        if direct {
            r.skip(4)?;
        }
        let mut pm = PixMap::read(r, direct, direct)?;
        // BitsRect and BitsRgn store their rows unpacked.
        pm.packed = op >= 0x98;
        let (src, dst, mode) = (r.rect()?, r.rect()?, r.u16()?);
        let mask = if op & 1 != 0 { Some(self.read_region(r)?) } else { None };
        let (w, h) = pm.dims()?;
        let (wide, high) = (w as u64, h as u64);
        let fits = wide * high <= dib::MAX_READ_PIXELS && pm.row_len(w) as u64 * high <= MAX_ROW_BYTES;
        let readable = pm.decodable(w);
        let keep = fits && readable && self.charge(&pm, w, h);
        let (data, ok) = pm.rows(r, w, h, keep)?;
        if !keep {
            self.p.warn(if readable { TOO_MANY_PIXELS } else { BAD_IMAGE });
            return Some(());
        }
        let Some(mut img) = pm.to_rgba(&data, w, h, self.fg, self.bg).filter(|_| ok) else {
            self.p.warn(BAD_IMAGE);
            return Some(());
        };
        self.transfer(&mut img, mode, pm.size == 1 && pm.palette.is_none());
        if dst.width() <= 0.0 || dst.height() <= 0.0 || mask.as_ref().is_some_and(Vec::is_empty) {
            return Some(());
        }
        let saved = self.p.dc.clips.clone();
        if let Some(rects) = &mask {
            let mut bp = rects_path(rects);
            bp.apply_affine(self.p.xf());
            self.p.push_clip(bp, FillRule::NonZero, false);
        }
        self.p.image(Ok(img), src - pm.bounds.origin().to_vec2(), dst, SRCCOPY, 1.0);
        self.p.dc.clips = saved;
        Some(())
    }

    /// Transfer mode `mode` on copied pixels: OR of a bitmap and the transparent mode leave the
    /// background color out; the others are copied, with a warning.
    fn transfer(&mut self, img: &mut Rgba, mode: u16, bitmap: bool) {
        // 64 dithers, which doesn't change what is copied.
        let clear = match mode & !64 {
            0 => false,
            1 if bitmap => true,
            36 => true,
            _ => {
                self.p.warn(MODES);
                false
            }
        };
        if clear {
            let bg = self.bg;
            img.pixels.as_chunks_mut::<4>().0.iter_mut().filter(|p| [p[0], p[1], p[2]] == bg).for_each(|p| p[3] = 0);
        }
    }

    /// A compressed QuickTime image: a JPEG one becomes an image placed by its matrix.
    fn quicktime(&mut self, r: &mut Be) -> Option<()> {
        let n = usize::try_from(r.u32()?).ok()?;
        let data = r.bytes(n)?;
        match qt_image(data) {
            Some((ctype, xf, src, image)) if &ctype == b"jpeg" => match self.jpeg(image) {
                Ok(img) => {
                    let img = crop(img, src);
                    self.p.image_xf(img, xf * Affine::translate(src.origin().to_vec2()), 1.0);
                }
                Err(w) => self.p.warn(w),
            },
            Some(_) => self.p.warn(CODECS),
            None => self.p.warn(BAD_IMAGE),
        }
        Some(())
    }

    /// A JPEG stream's pixels, taken from the file's pixel budget.
    fn jpeg(&mut self, data: &[u8]) -> Result<Rgba, &'static str> {
        let reader = || image::ImageReader::with_format(Cursor::new(data), image::ImageFormat::Jpeg);
        let (w, h) = reader().into_dimensions().map_err(|_| BAD_IMAGE)?;
        if w > dib::MAX_READ_SIDE || h > dib::MAX_READ_SIDE || u64::from(w) * u64::from(h) > dib::MAX_READ_PIXELS {
            return Err(TOO_MANY_PIXELS);
        }
        dib::charge(w, h, &mut self.p.pixels_left).map_err(|_| TOO_MANY_PIXELS)?;
        let img = reader().decode().map_err(|_| BAD_IMAGE)?.to_rgba8();
        Ok(Rgba { width: img.width(), height: img.height(), pixels: img.into_raw() })
    }
}

/// A QuickTime image's compressor type, matrix (image → picture space), source rectangle and
/// compressed data, from its header and image description.
fn qt_image(d: &[u8]) -> Option<([u8; 4], Affine, Rect, &[u8])> {
    let mut b = Be::new(d, 2);
    let mut m = [0.0; 9];
    for v in &mut m {
        *v = f64::from(b.i32()?) / 65536.0;
    }
    let matte = usize::try_from(b.u32()?).ok()?;
    b.skip(10)?;
    let src = b.rect()?;
    b.skip(4)?;
    let mask = usize::try_from(b.u32()?).ok()?;
    b.skip(matte)?;
    b.skip(mask)?;
    let start = b.pos;
    let id_size = usize::try_from(b.u32()?).ok()?;
    let ctype = b.array::<4>()?;
    b.skip(36)?;
    let size = usize::try_from(b.u32()?).ok()?;
    let at = start.checked_add(id_size)?;
    let data = d.get(at..at.checked_add(size)?).or_else(|| d.get(at..))?;
    // The rows of a QuickTime matrix are [a b u] [c d v] [tx ty w]; u, v and w (perspective) are
    // left out.
    let xf = Affine::new([m[0], m[1], m[3], m[4], m[6], m[7]]);
    Some((ctype, xf, src, data))
}

/// Read a picture.
pub(crate) fn play(bytes: &[u8]) -> Result<Imported, String> {
    let h = find(bytes).ok_or(NOT_PICT)?;
    let mut q = Pict::new(h.frame);
    let mut r = Be::new(bytes, h.ops);
    // Why the picture stopped before its end opcode, if it did.
    let mut stopped = Some(TOO_MANY_OPS);
    for _ in 0..MAX_OPS {
        // Version 2 opcodes start on even bytes.
        if h.v2 && r.pos % 2 == 1 && r.skip(1).is_none() {
            stopped = Some(CUT_SHORT);
            break;
        }
        let Some(op) = (if h.v2 { r.u16() } else { r.u8().map(u16::from) }) else {
            stopped = Some(CUT_SHORT);
            break;
        };
        if op == 0x00FF {
            stopped = None;
            break;
        }
        if q.op(op, &mut r).is_none() {
            stopped = Some(CUT_SHORT);
            break;
        }
    }
    if let Some(why) = stopped {
        q.p.warn(why);
    }
    q.p.flush();
    if q.p.out.is_empty() {
        q.p.warn(NOTHING);
    }
    Ok(q.p.finish())
}
