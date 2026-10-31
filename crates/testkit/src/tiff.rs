//! Hand-written TIFF files for import tests: little-endian, uncompressed, 8 or 16 bits a sample,
//! with each pixel's samples stored together (chunky) or one plane per sample (planar,
//! PlanarConfiguration 2) (TIFF 6.0, sections 2 and 8); and 1-bit files compressed with CCITT
//! Group 3 (ITU-T T.4; Compression 2 and 3, T4Options and FillOrder as libtiff's `tiff.h` defines
//! them), coded by [`group3`].

use std::cell::RefCell;
use std::rc::Rc;

use fax::{BitWriter, Bits};

/// One image: its channels in the order the file stores them, color first, extra samples after.
#[derive(Clone, Debug, Default)]
pub struct Tiff<'a> {
    pub width: u32,
    pub height: u32,
    /// PhotometricInterpretation: 0 white is zero, 1 black is zero, 2 RGB, 5 separated (CMYK).
    pub photometric: u16,
    /// Bits a sample: 8 or 16.
    pub bits: u16,
    /// The ExtraSamples values, one for each channel after the color (0 not alpha, 1 associated
    /// alpha, 2 unassociated alpha); none writes no ExtraSamples tag.
    pub extra: &'a [u16],
    /// One plane per channel (PlanarConfiguration 2) instead of each pixel's samples together.
    pub planar: bool,
    /// Pixels per inch, written as XResolution and YResolution with ResolutionUnit 2.
    pub ppi: Option<u32>,
    /// The channels: `width` × `height` samples each, row by row.
    pub channels: &'a [Vec<u16>],
}

/// Pads `out` to an even length: TIFF offsets point at word boundaries.
fn align(out: &mut Vec<u8>) {
    if out.len() % 2 == 1 {
        out.push(0);
    }
}

/// A directory entry: its tag, field type, count and value bytes.
type Entry = (u16, u16, usize, Vec<u8>);

/// An entry of SHORT values.
fn shorts(tag: u16, v: &[u16]) -> Entry {
    (tag, 3, v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect())
}

/// An entry of LONG values.
fn longs(tag: u16, v: &[u32]) -> Entry {
    (tag, 4, v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect())
}

/// The TIFF file of `t`: the header, the directory at offset 8, the values longer than four
/// bytes, then the image in one strip a plane.
pub fn tiff(t: &Tiff) -> Vec<u8> {
    let pixels = usize::try_from(t.width * t.height).unwrap();
    assert!(t.channels.iter().all(|c| c.len() == pixels), "every channel has a sample a pixel");
    assert!(matches!(t.bits, 8 | 16), "8 or 16 bits a sample");
    let mut strips: Vec<Vec<u8>> = vec![];
    let samples_of = |i: usize| t.channels.iter().map(move |c| c[i]);
    let order: Vec<Vec<u16>> = if t.planar { t.channels.to_vec() } else { vec![(0..pixels).flat_map(samples_of).collect()] };
    for values in order {
        strips.push(match t.bits {
            8 => values.iter().map(|&v| u8::try_from(v).unwrap()).collect(),
            _ => values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        });
    }
    let samples = u16::try_from(t.channels.len()).unwrap();
    let counts: Vec<u32> = strips.iter().map(|s| u32::try_from(s.len()).unwrap()).collect();
    let entries = |offsets: &[u32]| -> Vec<Entry> {
        let rational = |tag, n: u32| (tag, 5, 1, [n.to_le_bytes(), 1u32.to_le_bytes()].concat());
        // In ascending tag order.
        let mut e = vec![
            longs(256, &[t.width]),
            longs(257, &[t.height]),
            shorts(258, &vec![t.bits; usize::from(samples)]),
            shorts(259, &[1]),
            shorts(262, &[t.photometric]),
            longs(273, offsets),
            shorts(277, &[samples]),
            longs(278, &[t.height]),
            longs(279, &counts),
        ];
        if let Some(ppi) = t.ppi {
            e.extend([rational(282, ppi), rational(283, ppi)]);
        }
        e.push(shorts(284, &[if t.planar { 2 } else { 1 }]));
        if t.ppi.is_some() {
            e.push(shorts(296, &[2]));
        }
        if !t.extra.is_empty() {
            e.push(shorts(338, t.extra));
        }
        e
    };
    file(entries, strips)
}

/// The TIFF file of the directory `entries` gives for the strips' offsets, and `strips`: the
/// header, the directory at offset 8, the values longer than four bytes, then the strips.
fn file(entries: impl Fn(&[u32]) -> Vec<Entry>, strips: Vec<Vec<u8>>) -> Vec<u8> {
    // The strips' offsets follow from the sizes of the rest, which don't depend on them.
    let padded = |n: usize| n + n % 2;
    let sized = entries(&vec![0; strips.len()]);
    let ifd_len = 2 + 12 * sized.len() + 4;
    let values_len: usize = sized.iter().filter(|e| e.3.len() > 4).map(|e| padded(e.3.len())).sum();
    let mut at = 8 + ifd_len + values_len;
    let mut offsets = vec![];
    for s in &strips {
        offsets.push(u32::try_from(at).unwrap());
        at += padded(s.len());
    }
    let mut out = b"II*\0\x08\0\0\0".to_vec();
    let entries = entries(&offsets);
    out.extend(u16::try_from(entries.len()).unwrap().to_le_bytes());
    let mut values = vec![];
    for (tag, ty, count, bytes) in entries {
        out.extend(tag.to_le_bytes());
        out.extend(ty.to_le_bytes());
        out.extend(u32::try_from(count).unwrap().to_le_bytes());
        if bytes.len() <= 4 {
            out.extend(&bytes);
            out.extend(vec![0; 4 - bytes.len()]);
        } else {
            out.extend(u32::try_from(8 + ifd_len + values.len()).unwrap().to_le_bytes());
            values.extend(bytes);
            align(&mut values);
        }
    }
    out.extend([0; 4]);
    out.extend(values);
    for s in strips {
        out.extend(s);
        align(&mut out);
    }
    assert_eq!(out.len(), at, "the strips sit where the directory says");
    out
}

/// A 1-bit image compressed with CCITT Group 3, in strips of `rows_per_strip` rows or in tiles,
/// each coded on its own.
#[derive(Clone, Debug)]
pub struct Group3<'a> {
    /// The pixels, row by row, `true` for a black run's.
    pub rows: &'a [Vec<bool>],
    /// Compression: 2 (modified Huffman: each row starts at a byte, no EOL codes) or 3 (T.4: an
    /// EOL code before each row).
    pub compression: u16,
    /// T4Options, written with Compression 3: bit 0 codes each row after a strip's first against
    /// the row above (two-dimensional), after a tag bit; bit 2 adds fill bits so that each EOL
    /// code ends at a byte.
    pub t4_options: u32,
    /// PhotometricInterpretation: 0 white is zero, 1 black is zero.
    pub photometric: u16,
    /// FillOrder: 1 (written as no tag) or 2 (each byte's lowest bit first).
    pub fill_order: u16,
    pub rows_per_strip: u32,
    /// Tiles of this width and height instead of strips (TileWidth, TileLength, TileOffsets and
    /// TileByteCounts), left to right and top to bottom; their pixels past the image are white.
    pub tile: Option<(u32, u32)>,
    /// With Compression 3, the last strip or tile ends with six EOL codes (T.4's return to
    /// control).
    pub rtc: bool,
}

/// The bits a fax encoder writes, in order.
#[derive(Clone, Default)]
struct Written(Rc<RefCell<Vec<Bits>>>);

impl BitWriter for Written {
    type Error = std::convert::Infallible;

    fn write(&mut self, bits: Bits) -> Result<(), Self::Error> {
        self.0.borrow_mut().push(bits);
        Ok(())
    }
}

/// `row` as one-dimensional runs (modified Huffman), white and black in turn from a white one
/// (ITU-T T.4 §4.1.1); runs of 64 to 1791 pixels take a make-up code.
fn runs(row: &[bool]) -> Vec<Bits> {
    let mut out = vec![];
    let (mut x, mut black) = (0, false);
    while x < row.len() {
        let n = row[x..].iter().take_while(|b| **b == black).count();
        let code = |n: usize| if black { fax::maps::black::encode(n as u16) } else { fax::maps::white::encode(n as u16) }.unwrap();
        if n >= 64 {
            out.push(code(n & !63));
        }
        out.push(code(n % 64));
        (x, black) = (x + n, !black);
    }
    out
}

/// The coded data of the strip or tile of `g` that holds `rows`; the `last` one carries the six
/// EOL codes [`Group3::rtc`] asks for.
fn group3_strip(g: &Group3, rows: &[Vec<bool>], last: bool) -> Vec<u8> {
    let two_d = g.compression == 3 && g.t4_options & 1 != 0;
    let fill = g.compression == 3 && g.t4_options & 4 != 0;
    let written = Written::default();
    let mut encoder = fax::encoder::Encoder::new(written.clone());
    let mut bits: Vec<Bits> = vec![];
    let used = |bits: &[Bits]| bits.iter().map(|b| usize::from(b.len)).sum::<usize>();
    let zeros = |n: usize| Bits { data: 0, len: n as u8 };
    let eol = |bits: &mut Vec<Bits>, tag: Option<bool>| {
        // Fill bits so that the EOL code (12 bits) ends at a byte.
        let pad = (8 - (used(bits) + 12) % 8) % 8;
        if fill && pad > 0 {
            bits.push(zeros(pad));
        }
        bits.push(fax::maps::EOL);
        if let Some(one_d) = tag {
            bits.push(Bits { data: u16::from(one_d), len: 1 });
        }
    };
    for (i, row) in rows.iter().enumerate() {
        // The encoder codes each row against the one above (as T.6 and T.4's two-dimensional rows
        // are coded), the first against a white row.
        let at = written.0.borrow().len();
        encoder.encode_line(row.iter().map(|b| if *b { fax::Color::Black } else { fax::Color::White }), row.len() as u16).unwrap();
        let coded = written.0.borrow()[at..].to_vec();
        if g.compression == 2 {
            let pad = (8 - used(&bits) % 8) % 8;
            if pad > 0 {
                bits.push(zeros(pad));
            }
        } else {
            eol(&mut bits, two_d.then_some(i == 0));
        }
        bits.extend(if two_d && i > 0 { coded } else { runs(row) });
    }
    if g.compression == 3 && g.rtc && last {
        for _ in 0..6 {
            eol(&mut bits, two_d.then_some(true));
        }
    }
    let mut w = fax::VecWriter::new();
    for b in bits {
        w.write(b).unwrap();
    }
    let data = w.finish();
    if g.fill_order == 2 { data.iter().map(|b| b.reverse_bits()).collect() } else { data }
}

/// The TIFF file of `g`, its rows coded by [`group3_strip`].
pub fn group3(g: &Group3) -> Vec<u8> {
    let height = u32::try_from(g.rows.len()).unwrap();
    let width = u32::try_from(g.rows[0].len()).unwrap();
    assert!(g.rows.iter().all(|r| r.len() == g.rows[0].len()), "every row has the same width");
    assert!(matches!(g.compression, 2 | 3), "Compression 2 or 3");
    let chunks: Vec<Vec<Vec<bool>>> = match g.tile {
        Some((tw, th)) => {
            let (across, down) = (width.div_ceil(tw), height.div_ceil(th));
            let px = |x: u32, y: u32| g.rows.get(y as usize).and_then(|r| r.get(x as usize)).copied().unwrap_or(false);
            let tile = |tx: u32, ty: u32| (0..th).map(|r| (0..tw).map(|c| px(tx * tw + c, ty * th + r)).collect()).collect();
            (0..down).flat_map(|ty| (0..across).map(move |tx| (tx, ty))).map(|(tx, ty)| tile(tx, ty)).collect()
        }
        None => g.rows.chunks(usize::try_from(g.rows_per_strip).unwrap()).map(<[Vec<bool>]>::to_vec).collect(),
    };
    let strips: Vec<Vec<u8>> = chunks.iter().enumerate().map(|(i, rows)| group3_strip(g, rows, i + 1 == chunks.len())).collect();
    let counts: Vec<u32> = strips.iter().map(|s| u32::try_from(s.len()).unwrap()).collect();
    let entries = |offsets: &[u32]| -> Vec<Entry> {
        // In ascending tag order.
        let mut e =
            vec![longs(256, &[width]), longs(257, &[height]), shorts(258, &[1]), shorts(259, &[g.compression]), shorts(262, &[g.photometric])];
        if g.fill_order != 1 {
            e.push(shorts(266, &[g.fill_order]));
        }
        if g.tile.is_none() {
            e.push(longs(273, offsets));
        }
        e.push(shorts(277, &[1]));
        if g.tile.is_none() {
            e.extend([longs(278, &[g.rows_per_strip]), longs(279, &counts)]);
        }
        if g.compression == 3 {
            e.push(longs(292, &[g.t4_options]));
        }
        if let Some((tw, th)) = g.tile {
            e.extend([longs(322, &[tw]), longs(323, &[th]), longs(324, offsets), longs(325, &counts)]);
        }
        e
    };
    file(entries, strips)
}
