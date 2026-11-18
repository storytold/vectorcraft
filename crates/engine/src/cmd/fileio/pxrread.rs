//! Reading Pixar picture files (`.pxr`) for File › Open and File › Place, from "The Format of
//! Stored Pictures" in the Pixar Image Computer manuals (release 1.2, December 1986). [`is_pxr`]
//! recognizes a file by the magic number at the start of its 512-byte header, and [`decode`] reads
//! its tiles into RGBA pixels: any subset of the R, G, B and A channels, 8 or 12 bits a sample,
//! dumped or run-length encoded, with alpha matted to black or unassociated. The file is
//! untrusted: every read is checked.
//!
//! Display rules for what the format leaves open: a picture whose only color channel is R is
//! gray; a missing color channel is 0 and a missing A is opaque; a null tile is transparent when
//! the picture has A and black otherwise. The x and y offsets are not read.

/// The size of the header, after which the tile pointer table starts.
const HEADER: usize = 512;

/// The first four bytes of every picture: the magic number `0x0000E880`, least significant byte
/// first.
const MAGIC: [u8; 4] = [0x80, 0xe8, 0x00, 0x00];

/// The message for a file whose header, tile table or tile data ends early, or that holds a tile
/// whose writing was not finished.
const TRUNCATED: &str = "the Pixar file is cut short";

/// The message for a header whose sizes, channels, storage mode or alpha mode the format doesn't
/// define.
const DAMAGED: &str = "the Pixar file's header is damaged";

/// The message for a tile pointer into the header or the tile pointer table, and for encoded tile
/// data with a packet the format doesn't define, or one that runs past the end of its scan line.
const BAD_DATA: &str = "the Pixar file's image data is damaged";

/// The channel bits of the picture format field (header byte 424): R, G, B and A, stored in this
/// order in each pixel.
const CHANNELS: [u16; 4] = [8, 4, 2, 1];

/// Does `bytes` start with a Pixar picture header's magic number?
pub(super) fn is_pxr(bytes: &[u8]) -> bool {
    bytes.starts_with(&MAGIC)
}

/// How one tile's samples are stored and how they become pixels.
struct Layout {
    /// The channel indices (0 R, 1 G, 2 B, 3 A) the file stores, in the order it stores them.
    stored: Vec<usize>,
    /// Is a sample 12 bits in two bytes, rather than 8 bits in one?
    wide: bool,
    /// The bytes of one stored sample.
    sample: usize,
    /// Is the only color channel R, so the picture is gray?
    gray: bool,
    /// Are R, G and B matted to black (multiplied by A)?
    matted: bool,
    /// Does the picture store A?
    alpha: bool,
}

impl Layout {
    /// The display value of the sample at `at` in `bytes`. A 12-bit sample is the low 12 bits of a
    /// two-byte value, where 0 is 0.0, 2048 is 1.0, 2049 to 3071 are past 1.0 and 3072 to 4095 are
    /// below 0.0; values outside 0.0 to 1.0 are clamped.
    fn sample(&self, bytes: &[u8], at: usize) -> Result<u8, String> {
        if !self.wide {
            return bytes.get(at).copied().ok_or_else(|| TRUNCATED.to_string());
        }
        let pair = bytes.get(at..at + 2).ok_or(TRUNCATED)?;
        let v = u32::from(u16::from_le_bytes([pair[0], pair[1]]) & 0x0fff);
        let v = if v >= 3072 { 0 } else { v.min(2048) };
        Ok(((v * 255 + 1024) / 2048) as u8)
    }

    /// The RGBA pixel of the samples for `channels` read from `bytes` at `at`, combined with
    /// `base` (the channels not read, such as a packet's common A).
    fn pixel(&self, bytes: &[u8], at: usize, channels: &[usize], mut base: [u8; 4]) -> Result<[u8; 4], String> {
        for (i, &c) in channels.iter().enumerate() {
            base[c] = self.sample(bytes, at + i * self.sample)?;
        }
        Ok(base)
    }

    /// The color a pixel of stored samples is shown in: gray from R, and straight color from color
    /// matted to black (0 where A is 0).
    fn shown(&self, [r, g, b, a]: [u8; 4]) -> [u8; 4] {
        let [r, g, b] = if self.gray { [r; 3] } else { [r, g, b] };
        if !self.alpha {
            return [r, g, b, 255];
        }
        if !self.matted {
            return [r, g, b, a];
        }
        let unmatte = |c: u8| if a == 0 { 0 } else { ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8 };
        [unmatte(r), unmatte(g), unmatte(b), a]
    }
}

/// The pixels of Pixar picture `bytes`, which [`is_pxr`] accepts, refused when they take more
/// than `max_alloc` bytes as RGBA before anything is decoded.
pub(super) fn decode(bytes: &[u8], max_alloc: u64) -> Result<image::RgbaImage, String> {
    let head = bytes.get(..HEADER).ok_or(TRUNCATED)?;
    let le = |at: usize| u16::from_le_bytes([head[at], head[at + 1]]);
    let (h, w, tile_h, tile_w) = (le(416), le(418), le(420), le(422));
    let (format, storage, block, alpha_mode) = (le(424), le(426), le(428), le(430));
    // Sizes must be positive; the format field holds only the four channel bits; the storage modes
    // are 0 to 3 and the alpha modes 0 and 1.
    if w == 0 || h == 0 || tile_w == 0 || tile_h == 0 {
        return Err(DAMAGED.into());
    }
    if format == 0 || format > 15 || storage > 3 || alpha_mode > 1 {
        return Err(DAMAGED.into());
    }
    let (w, h, tile_w, tile_h) = (usize::from(w), usize::from(h), usize::from(tile_w), usize::from(tile_h));
    if w as u64 * h as u64 * 4 > max_alloc {
        return Err(format!("an image of {w} × {h} pixels is too large to draw"));
    }
    let stored: Vec<usize> = CHANNELS.iter().enumerate().filter(|(_, bit)| format & **bit != 0).map(|(i, _)| i).collect();
    let wide = storage % 2 == 1;
    let layout =
        Layout { gray: format & 0b1110 == 0b1000, matted: alpha_mode == 0, alpha: format & 1 != 0, sample: if wide { 2 } else { 1 }, wide, stored };
    let (across, down) = (w.div_ceil(tile_w), h.div_ceil(tile_h));
    // A table end past the largest address is past the end of the file too.
    let table_end = across.checked_mul(down).and_then(|n| n.checked_mul(8)).and_then(|n| n.checked_add(HEADER)).ok_or(TRUNCATED)?;
    let table = bytes.get(HEADER..table_end).ok_or(TRUNCATED)?;
    let empty = if layout.alpha { [0; 4] } else { [0, 0, 0, 255] };
    let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba(empty));
    // The encoded data read in all, which a file whose tiles each hold their own data stays within.
    let mut left = bytes.len();
    for (n, entry) in table.as_chunks::<8>().0.iter().enumerate() {
        let pointer = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]) as usize;
        let length = i32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
        if pointer == 0 {
            continue;
        }
        // A tile that starts past the end of the file; every offset read from here stays near the
        // file's length, so it fits a 32-bit address.
        if length == -1 || pointer >= bytes.len() {
            return Err(TRUNCATED.into());
        }
        // Tile data follows the tile pointer table; a pointer into the header or the table would
        // read them as pixels.
        if pointer < table_end {
            return Err(BAD_DATA.into());
        }
        let (x0, y0) = (n % across * tile_w, n / across * tile_h);
        // The part of the tile inside the picture; the pixels of border tiles past its edges are
        // stored and skipped.
        let tile = Tile { w: tile_w, h: tile_h, shown_w: tile_w.min(w - x0), shown_h: tile_h.min(h - y0) };
        let mut put = |x: usize, y: usize, px: [u8; 4]| {
            img.put_pixel((x0 + x) as u32, (y0 + y) as u32, image::Rgba(layout.shown(px)));
        };
        if storage >= 2 {
            dumped(bytes, pointer, tile, &layout, &mut put)?;
        } else {
            encoded(bytes, pointer, tile, usize::from(block), &layout, &mut left, &mut put)?;
        }
    }
    Ok(img)
}

/// A tile's size, and the size of its part inside the picture (the pixels that are drawn).
#[derive(Clone, Copy)]
struct Tile {
    w: usize,
    h: usize,
    shown_w: usize,
    shown_h: usize,
}

/// Read a dumped tile at `at`: its pixels row by row, each pixel's samples in channel order. The
/// whole tile must be in the file; only its part inside the picture is read.
fn dumped(bytes: &[u8], at: usize, tile: Tile, layout: &Layout, put: &mut impl FnMut(usize, usize, [u8; 4])) -> Result<(), String> {
    let size = layout.stored.len() * layout.sample;
    let end = tile.w.checked_mul(tile.h).and_then(|n| n.checked_mul(size)).and_then(|n| n.checked_add(at)).ok_or(TRUNCATED)?;
    if end > bytes.len() {
        return Err(TRUNCATED.into());
    }
    for y in 0..tile.shown_h {
        for x in 0..tile.shown_w {
            put(x, y, layout.pixel(bytes, at + (y * tile.w + x) * size, &layout.stored, [0, 0, 0, 255])?);
        }
    }
    Ok(())
}

/// Read an encoded tile at `at`: each scan line is a series of packets, each headed by a 16-bit
/// value whose top 4 bits are its kind and whose low 12 bits are its count less one.
///
/// - 0 ends the disk block: reading goes on at the next multiple of `block` bytes in the file.
/// - 1 dumps count + 1 pixels with every channel.
/// - 2 holds count + 1 runs, each a length byte (pixels less one) and one pixel with every channel.
/// - 3 dumps count + 1 pixels' color channels after one A sample they share.
/// - 4 holds count + 1 runs of color channels, each a length byte and one pixel, after one A
///   sample they share.
///
/// No packet runs past the end of its scan line, and a block end never follows another one (an
/// empty block). Reading stops after the last scan line inside the picture, pixels past its right
/// edge are skipped, and the bytes read count against `left`, the encoded data left to read in
/// the whole file: tiles that share their data are refused once it is used up.
fn encoded(
    bytes: &[u8],
    mut at: usize,
    tile: Tile,
    block: usize,
    layout: &Layout,
    left: &mut usize,
    put: &mut impl FnMut(usize, usize, [u8; 4]),
) -> Result<(), String> {
    let all = &layout.stored;
    let colors: Vec<usize> = all.iter().copied().filter(|c| *c != 3).collect();
    let mut block_ended = false;
    for y in 0..tile.shown_h {
        let mut x = 0;
        while x < tile.w {
            let start = at;
            let head = bytes.get(at..at + 2).ok_or(TRUNCATED)?;
            let head = u16::from_le_bytes([head[0], head[1]]);
            let (kind, count) = (head >> 12, usize::from(head & 0x0fff) + 1);
            if kind == 0 {
                if block == 0 || block_ended {
                    return Err(BAD_DATA.into());
                }
                block_ended = true;
                at = (at / block + 1) * block;
                if at >= bytes.len() {
                    return Err(TRUNCATED.into());
                }
                *left = left.checked_sub(at - start).ok_or(BAD_DATA)?;
                continue;
            }
            block_ended = false;
            at += 2;
            let shared = matches!(kind, 3 | 4);
            if shared && (!layout.alpha || colors.is_empty()) {
                return Err(BAD_DATA.into());
            }
            let (channels, base) = if shared {
                let a = layout.sample(bytes, at)?;
                at += layout.sample;
                (&colors, [0, 0, 0, a])
            } else {
                (all, [0, 0, 0, 255])
            };
            let size = channels.len() * layout.sample;
            match kind {
                1 | 3 => {
                    if x + count > tile.w {
                        return Err(BAD_DATA.into());
                    }
                    let end = at + count * size;
                    if end > bytes.len() {
                        return Err(TRUNCATED.into());
                    }
                    for i in 0..count.min(tile.shown_w.saturating_sub(x)) {
                        put(x + i, y, layout.pixel(bytes, at + i * size, channels, base)?);
                    }
                    at = end;
                    x += count;
                }
                2 | 4 => {
                    for _ in 0..count {
                        let run = usize::from(*bytes.get(at).ok_or(TRUNCATED)?) + 1;
                        if x + run > tile.w {
                            return Err(BAD_DATA.into());
                        }
                        let px = layout.pixel(bytes, at + 1, channels, base)?;
                        at += 1 + size;
                        for i in 0..run.min(tile.shown_w.saturating_sub(x)) {
                            put(x + i, y, px);
                        }
                        x += run;
                    }
                }
                _ => return Err(BAD_DATA.into()),
            }
            *left = left.checked_sub(at - start).ok_or(BAD_DATA)?;
        }
    }
    Ok(())
}
