//! vectorcraft-doc's Group 3 TIFF reader (`cmyk::group3_tiff_rgba`) reads CCITT data through
//! craft-stream-codecs; this keeps the reader it had on hayro-ccitt as the reference it matches.

use vectorcraft_doc::cmyk::{Group3Error, group3_tiff_rgba};
use vectorcraft_testkit::tiff::{Group3, group3};

/// What the reference refused a file for, as [`Group3Error`] says it.
#[derive(Debug, PartialEq)]
enum Refused {
    TooLarge,
    Damaged(craft_stream_codecs::CcittErrorKind),
}

fn kind_of(e: hayro_ccitt::DecodeError) -> craft_stream_codecs::CcittErrorKind {
    use craft_stream_codecs::CcittErrorKind as K;
    match e {
        hayro_ccitt::DecodeError::UnexpectedEof => K::UnexpectedEof,
        hayro_ccitt::DecodeError::Overflow => K::Overflow,
        _ => K::InvalidData,
    }
}

/// `group3_tiff_rgba` as it read before craft-stream-codecs.
fn reference(bytes: &[u8]) -> Option<Result<image::RgbaImage, Refused>> {
    use hayro_ccitt::{DecodeError, DecodeSettings, DecoderContext, EncodingMode};
    use tiff::tags::Tag;
    if !(bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")) || vectorcraft_doc::cmyk::oversized_tiff(bytes).is_some() {
        return None;
    }
    let mut d = tiff::decoder::Decoder::new(std::io::Cursor::new(bytes)).ok()?;
    let kind = d.colortype().ok()?;
    let compression = d.find_tag_unsigned::<u16>(Tag::Compression).ok().flatten()?;
    let (w, h) = d.dimensions().ok()?;
    if kind != tiff::ColorType::Gray(1) || !matches!(compression, 2 | 3) || w == 0 || h == 0 {
        return None;
    }
    // The level of a 0 bit (a white run): white when white is zero (0), black when black is zero (1).
    let zero = match d.find_tag_unsigned::<u16>(Tag::PhotometricInterpretation).ok().flatten() {
        Some(0) => u8::MAX,
        Some(1) => 0,
        _ => return None,
    };
    if u64::from(w) * u64::from(h) > vectorcraft_doc::MAX_RASTER_ALLOC / 4 {
        return Some(Err(Refused::TooLarge));
    }
    let t4_options = d.find_tag_unsigned::<u32>(Tag::Unknown(292)).ok().flatten().unwrap_or(0);
    let lsb_first = d.find_tag_unsigned::<u16>(Tag::FillOrder).ok().flatten() == Some(2);
    let encoding = if compression == 3 && t4_options & 1 != 0 {
        // Each row's tag bit says how it is coded.
        EncodingMode::Group3_2D { k: 2 }
    } else {
        EncodingMode::Group3_1D
    };
    // The width and height of each strip or tile, and the tags of their offsets and byte counts.
    let tiled = d.get_chunk_type() == tiff::decoder::ChunkType::Tile;
    let (cw, ch, offsets, counts) = if tiled {
        let tw = d.find_tag_unsigned::<u32>(Tag::TileWidth).ok().flatten().filter(|&v| v > 0)?;
        let th = d.find_tag_unsigned::<u32>(Tag::TileLength).ok().flatten().filter(|&v| v > 0)?;
        (tw, th, Tag::TileOffsets, Tag::TileByteCounts)
    } else {
        let rows = d.find_tag_unsigned::<u32>(Tag::RowsPerStrip).ok().flatten().filter(|&v| v > 0).map_or(h, |r| r.min(h));
        (w, rows, Tag::StripOffsets, Tag::StripByteCounts)
    };
    let damaged = |e: DecodeError| Some(Err(Refused::Damaged(kind_of(e))));
    // A tile is held to the allowance the image is held to; a strip is no larger than the image.
    if u64::from(cw) * u64::from(ch) > vectorcraft_doc::MAX_RASTER_ALLOC / 4 {
        return damaged(DecodeError::Overflow);
    }
    let offsets = d.find_tag_unsigned_vec::<u64>(offsets).ok().flatten().unwrap_or_default();
    let counts = d.find_tag_unsigned_vec::<u64>(counts).ok().flatten().unwrap_or_default();
    let across = w.div_ceil(cw);
    let chunks = u64::from(across) * u64::from(h.div_ceil(ch));
    if chunks > offsets.len().min(counts.len()) as u64 {
        return damaged(DecodeError::UnexpectedEof);
    }
    let mut gray = vec![zero; usize::try_from(u64::from(w) * u64::from(h)).ok()?];
    let mut budget = bytes.len();
    for (i, (&offset, &count)) in (0..chunks).zip(offsets.iter().zip(&counts)) {
        let (x0, y0) = (u32::try_from(i % u64::from(across)).ok()? * cw, u32::try_from(i / u64::from(across)).ok()? * ch);
        let Some(start) = usize::try_from(offset).ok().filter(|&s| s <= bytes.len()) else {
            return damaged(DecodeError::UnexpectedEof);
        };
        let len = usize::try_from(count).unwrap_or(usize::MAX).min(bytes.len() - start).min(budget);
        budget -= len;
        let data = bytes.get(start..start + len)?;
        let data: std::borrow::Cow<[u8]> = if lsb_first { data.iter().map(|b| b.reverse_bits()).collect() } else { data.into() };
        let settings = DecodeSettings {
            columns: cw,
            // A tile is coded whole; the last strip only has the image's remaining rows.
            rows: if tiled { ch } else { ch.min(h - y0) },
            end_of_block: compression == 3,
            end_of_line: compression == 3,
            rows_are_byte_aligned: compression == 2,
            encoding,
            invert_black: false,
        };
        let mut rows = Rows { out: &mut gray, width: w, height: h, x0, y0, x: 0, y: 0, zero };
        if let Err(e) = hayro_ccitt::decode(&data, &mut rows, &mut DecoderContext::new(settings)) {
            return damaged(e);
        }
        // The data ended, or six EOL codes came, before the strip's or tile's last row.
        if rows.y < settings.rows {
            return damaged(DecodeError::UnexpectedEof);
        }
    }
    let gray = image::GrayImage::from_raw(w, h, gray)?;
    Some(Ok(image::DynamicImage::ImageLuma8(gray).to_rgba8()))
}

/// Where a strip or tile of a Group 3 TIFF ([`group3_tiff_rgba`]) puts its decoded pixels: `out`
/// holds the image's levels row by row, `width` × `height`, and the strip or tile starts at (`x0`,
/// `y0`). Pixels past the image's right or bottom edge (a tile's padding) are dropped.
struct Rows<'a> {
    out: &'a mut [u8],
    width: u32,
    height: u32,
    x0: u32,
    y0: u32,
    /// The next pixel in the strip or tile.
    x: u64,
    y: u32,
    /// The level of a white run.
    zero: u8,
}

impl Rows<'_> {
    /// The next `n` pixels of the row, a white run's or a black run's.
    fn run(&mut self, white: bool, n: u64) {
        let level = if white { self.zero } else { u8::MAX - self.zero };
        let row = u64::from(self.y0) + u64::from(self.y);
        let left = u64::from(self.x0) + self.x;
        let right = left.saturating_add(n).min(u64::from(self.width));
        if row < u64::from(self.height) && left < right {
            let at = row * u64::from(self.width);
            if let (Ok(a), Ok(b)) = (usize::try_from(at + left), usize::try_from(at + right))
                && let Some(px) = self.out.get_mut(a..b)
            {
                px.fill(level);
            }
        }
        self.x = self.x.saturating_add(n);
    }
}

impl hayro_ccitt::Decoder for Rows<'_> {
    fn push_pixel(&mut self, white: bool) {
        self.run(white, 1);
    }

    fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
        self.run(white, u64::from(chunk_count) * 8);
    }

    fn next_line(&mut self) {
        self.y = self.y.saturating_add(1);
        self.x = 0;
    }
}

fn read(bytes: &[u8]) -> Option<Result<image::RgbaImage, Refused>> {
    group3_tiff_rgba(bytes).map(|r| {
        r.map_err(|e| match e {
            Group3Error::TooLarge { .. } => Refused::TooLarge,
            Group3Error::Damaged(k) => Refused::Damaged(k),
        })
    })
}

/// Group 3 TIFFs of every kind the reader takes: Compression 2 and 3, one- and two-dimensional
/// rows, fill bits, each byte's lowest bit first, white or black as zero, strips of a few rows
/// or one, tiles, with and without six EOLs at the end; their rows of many patterns and widths.
fn samples() -> Vec<Vec<u8>> {
    let mut files = Vec::new();
    for (width, height) in [(1usize, 1usize), (7, 3), (40, 6), (64, 17), (131, 9)] {
        let rows: Vec<Vec<bool>> = (0..height).map(|y| (0..width).map(|x| (x * (y + 1) / 3 + y + x / 7) % 4 < 2).collect()).collect();
        for (compression, t4_options) in [(2u16, 0u32), (3, 0), (3, 1), (3, 4), (3, 5)] {
            for (photometric, fill_order) in [(0u16, 1u16), (1, 2)] {
                for (rows_per_strip, tile) in [(1u32, None), (2, None), (64, None), (64, Some((16u32, 16u32)))] {
                    if tile.is_some() && compression == 2 {
                        continue;
                    }
                    let rtc = compression == 3 && rows_per_strip != 2;
                    let g = Group3 { rows: &rows, compression, t4_options, photometric, fill_order, rows_per_strip, tile, rtc };
                    files.push(group3(&g));
                }
            }
        }
    }
    files
}

/// Every sample, and every sample cut short or with bytes changed, reads to the same pixels or is
/// refused for the same reason as with the reference; strips or tiles wider than
/// `craft_stream_codecs::MAX_CCITT_COLUMNS` (2^20) are refused as an overflow where the reference
/// gave another reason.
#[test]
fn group3_tiffs_read_as_the_hayro_reader_read_them() {
    let files = samples();
    use craft_stream_codecs::CcittErrorKind::Overflow;
    let (mut same, mut refused, mut checked, mut wider) = (0, 0, 0, 0);
    for (n, file) in files.iter().enumerate() {
        let theirs = reference(file);
        assert!(theirs.as_ref().is_some_and(|r| r.is_ok()), "sample {n} reads");
        let mut variants = vec![file.clone()];
        for cut in [file.len() - 1, file.len() - 3, file.len() * 3 / 4, file.len() / 2] {
            variants.push(file[..cut].to_vec());
        }
        for k in 0..24usize {
            let mut v = file.clone();
            let at = (k * 2_654_435_761 + n * 97) % v.len();
            v[at] ^= 1 << (k % 8);
            variants.push(v);
        }
        for v in variants {
            let (ours, theirs) = (read(&v), reference(&v));
            match (&ours, &theirs) {
                (Some(Ok(a)), Some(Ok(b))) => {
                    assert!(a == b, "sample {n}: other pixels");
                    same += 1;
                }
                _ => {
                    let (ours, theirs) = (ours.map(|r| r.err()), theirs.map(|r| r.err()));
                    // A strip or tile wider than craft-stream-codecs reads (MAX_CCITT_COLUMNS)
                    // is refused as an overflow before decoding; hayro-ccitt ran out of data or
                    // found a bad code in it instead.
                    let wide = matches!((&ours, &theirs), (Some(Some(Refused::Damaged(Overflow))), Some(Some(Refused::Damaged(_)))));
                    assert!(ours == theirs || wide, "sample {n}: {ours:?}, reference {theirs:?}");
                    wider += usize::from(ours != theirs);
                    refused += 1;
                }
            }
            checked += 1;
        }
    }
    assert!(
        same > files.len() && refused > files.len() && wider < refused / 10,
        "{same} read, {refused} refused ({wider} as wider than read) of {checked}"
    );
}
