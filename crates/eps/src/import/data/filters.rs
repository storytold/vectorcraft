use super::ops::MAX_DATA;
use super::*;

/// A file object: the program itself, or data a filter decoded.
#[derive(Debug)]
pub(crate) enum Stream {
    Current,
    /// Filters over the program's own data, decoded when first read.
    Pending(Vec<Filter>),
    Buf {
        data: Vec<u8>,
        pos: usize,
        /// The samples are 8-bit gray, RGB or CMYK (1, 3 or 4 channels) whatever the image's
        /// color space: a decoded JPEG.
        rgb: Option<u8>,
    },
}

pub(super) fn io<T>(at: &str) -> Res<T> {
    ps_err("ioerror", at)
}

/// The bytes `RunLengthDecode` makes of `raw`, and how many it read.
fn run_length(raw: &[u8]) -> Res<(Vec<u8>, usize)> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(&n) = raw.get(i) {
        i += 1;
        match n {
            128 => break,
            0..=127 => {
                let len = usize::from(n) + 1;
                out.extend_from_slice(raw.get(i..i + len).ok_or(PsError::Ps("ioerror", "RunLengthDecode".into()))?);
                i += len;
            }
            _ => {
                let b = *raw.get(i).ok_or(PsError::Ps("ioerror", "RunLengthDecode".into()))?;
                i += 1;
                out.extend(std::iter::repeat_n(b, 257 - usize::from(n)));
            }
        }
        if out.len() > MAX_DATA {
            return Err(PsError::Limit("decoded data is too large"));
        }
    }
    Ok((out, i))
}

/// zlib data inflated, and how many bytes of `raw` it took.
fn flate(raw: &[u8]) -> Res<(Vec<u8>, usize)> {
    let mut z = flate2::Decompress::new(true);
    let mut out = Vec::with_capacity(raw.len().saturating_mul(3).min(MAX_DATA));
    loop {
        let before = (z.total_in(), z.total_out());
        if out.capacity() - out.len() < 64 * 1024 {
            out.reserve(256 * 1024);
        }
        let input = raw.get(z.total_in() as usize..).unwrap_or_default();
        let status = z.decompress_vec(input, &mut out, flate2::FlushDecompress::None).map_err(|_| PsError::Ps("ioerror", "FlateDecode".into()))?;
        if out.len() > MAX_DATA {
            return Err(PsError::Limit("decoded data is too large"));
        }
        match status {
            flate2::Status::StreamEnd => break,
            _ if (z.total_in(), z.total_out()) == before => break,
            _ => {}
        }
    }
    Ok((out, z.total_in() as usize))
}

/// LZW data (`EarlyChange` 1) decoded, and how many bytes of `raw` it took.
fn lzw(raw: &[u8]) -> Res<(Vec<u8>, usize)> {
    let mut table: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
    table.push(vec![]);
    table.push(vec![]);
    let (mut out, mut prev): (Vec<u8>, Option<Vec<u8>>) = (vec![], None);
    let (mut acc, mut bits, mut width, mut i) = (0u32, 0u32, 9u32, 0usize);
    loop {
        while bits < width {
            let Some(&b) = raw.get(i) else { return Ok((out, i)) };
            i += 1;
            acc = (acc << 8) | u32::from(b);
            bits += 8;
        }
        let code = ((acc >> (bits - width)) & ((1 << width) - 1)) as usize;
        bits -= width;
        match code {
            256 => {
                table.truncate(258);
                width = 9;
                prev = None;
            }
            257 => return Ok((out, i)),
            _ => {
                let entry = match (table.get(code), &prev) {
                    (Some(e), _) if !e.is_empty() => e.clone(),
                    (_, Some(p)) if code == table.len() => {
                        let mut e = p.clone();
                        e.extend(p.first());
                        e
                    }
                    _ => return io("LZWDecode"),
                };
                out.extend_from_slice(&entry);
                if let Some(mut p) = prev.take()
                    && table.len() < 4096
                {
                    p.extend(entry.first());
                    table.push(p);
                }
                width = match table.len() + 1 {
                    n if n >= 2048 => 12,
                    n if n >= 1024 => 11,
                    n if n >= 512 => 10,
                    _ => 9,
                };
                prev = Some(entry);
                if out.len() > MAX_DATA {
                    return Err(PsError::Limit("decoded data is too large"));
                }
            }
        }
    }
}

/// A JPEG's pixels (gray, RGB or CMYK, 8 bits) with its channel count, and how many bytes of
/// `raw` it took. A CMYK JPEG gives its samples as stored, for the image's Decode array to read
/// (Adobe's CMYK JPEGs store them inverted, which a Decode array of `[1 0 1 0 1 0 1 0]` undoes).
fn dct(raw: &[u8]) -> Res<(Vec<u8>, usize, u8)> {
    let end = find(raw, &[0xFF, 0xD9]).map_or(raw.len(), |e| e + 2);
    let jpeg = raw.get(..end).unwrap_or_default();
    // `cmyk` gives the ink amounts of inverted samples: inverted back, they are the samples.
    if jpeg_components(jpeg) == Some(4)
        && let Some(inks) = ImageBlob::new("image/jpeg", jpeg.to_vec()).cmyk()
    {
        return Ok((inks.data.into_iter().map(|v| 255 - v).collect(), end, 4));
    }
    let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).map_err(|_| PsError::Ps("ioerror", "DCTDecode".into()))?;
    if img.color().channel_count() == 1 { Ok((img.to_luma8().into_raw(), end, 1)) } else { Ok((img.to_rgb8().into_raw(), end, 3)) }
}

/// CCITT fax data (ITU-T T.4 Group 3, one- or two-dimensional, and T.6 Group 4) decoded to rows
/// of 1-bit samples, each padded to a whole byte, and how many bytes of `raw` it took. With
/// `EndOfBlock` the rows end at the end-of-block code (EOFB, or RTC in Group 3) regardless of
/// `Rows`; without it they end after `Rows` rows, or at the end of the data when `Rows` is 0.
/// Damaged data ends the rows: those decoded before it are kept, and damage in the first row is an
/// `ioerror`. Rows that would take more than `MAX_DATA` bytes are refused.
fn fax(raw: &[u8], settings: DecodeSettings) -> Res<(Vec<u8>, usize)> {
    let row = (settings.columns as usize).div_ceil(8).max(1);
    let most = u32::try_from(MAX_DATA / row).unwrap_or(u32::MAX);
    if !settings.end_of_block && settings.rows > most {
        return Err(PsError::Limit("decoded data is too large"));
    }
    // Without a row count that ends the data, a row past `most` shows that the data is too large.
    let rows = if settings.end_of_block || settings.rows == 0 { most.saturating_add(1) } else { settings.rows };
    // The decoder reads the RTC of Group 3 two-dimensional data as rows: it gets the data before it.
    let rtc = match settings.encoding {
        EncodingMode::Group3_2D { .. } if settings.end_of_block => rtc(raw),
        _ => None,
    };
    let data = rtc.and_then(|(cut, _)| raw.get(..cut)).unwrap_or(raw);
    let mut out = FaxRows::default();
    let read = hayro_ccitt::decode(data, &mut out, &mut DecoderContext::new(DecodeSettings { rows, ..settings }));
    if out.rows > most {
        return Err(PsError::Limit("decoded data is too large"));
    }
    let used = match read {
        Ok(used) => used,
        Err(_) if out.rows > 0 => {
            out.data.truncate(out.rows as usize * row);
            // The whole rows decoded again, for how many bytes they took.
            let used = hayro_ccitt::decode(data, &mut Dropped, &mut DecoderContext::new(DecodeSettings { rows: out.rows, ..settings }));
            used.unwrap_or(data.len())
        }
        Err(_) => return io("CCITTFaxDecode"),
    };
    Ok((out.data, rtc.map_or(used, |(_, end)| end)))
}

/// The RTC that ends Group 3 two-dimensional data (six EOLs, each followed by a 1 tag bit, ITU-T
/// T.4 §4.2.4): the byte that holds the 1 bit of its first EOL, and the byte after its last tag
/// bit. The first EOL with a 1 tag bit that another EOL follows starts it.
fn rtc(raw: &[u8]) -> Option<(usize, usize)> {
    let bits = raw.len().saturating_mul(8);
    let bit = |i: usize| raw.get(i / 8).is_some_and(|b| b & (0x80 >> (i % 8)) != 0);
    let zeros = |i: usize| (i..bits).take_while(|j| !bit(*j)).count();
    // An EOL (its fill bits first) and a 1 tag bit from `i`: the EOL's 1 bit and where they end.
    let eol = |i: usize| {
        let one = i + zeros(i);
        (one - i >= 11 && bit(one) && bit(one + 1)).then_some((one, one + 2))
    };
    let mut i = 0;
    while i < bits {
        if let Some((one, next)) = eol(i)
            && let Some((_, mut end)) = eol(next)
        {
            for _ in 2..6 {
                match eol(end) {
                    Some((_, e)) => end = e,
                    None => break,
                }
            }
            return Some((one / 8, end.div_ceil(8)));
        }
        i += zeros(i) + 1;
    }
    None
}

/// Rows of 1-bit samples from a CCITT decoder, each padded to a whole byte: 1 is white, or black
/// with `BlackIs1`.
#[derive(Default)]
struct FaxRows {
    data: Vec<u8>,
    /// Samples in the row being decoded.
    x: u32,
    /// Whole rows decoded.
    rows: u32,
}

impl Decoder for FaxRows {
    fn push_pixel(&mut self, white: bool) {
        if self.x.is_multiple_of(8) {
            self.data.push(0);
        }
        if white && let Some(b) = self.data.last_mut() {
            *b |= 0x80 >> (self.x % 8);
        }
        self.x = self.x.saturating_add(1);
    }

    fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
        self.data.extend(std::iter::repeat_n(if white { 0xff } else { 0 }, chunk_count as usize));
        self.x = self.x.saturating_add(chunk_count.saturating_mul(8));
    }

    fn next_line(&mut self) {
        self.x = 0;
        self.rows = self.rows.saturating_add(1);
    }
}

/// A CCITT decoder that drops what it decodes.
struct Dropped;

impl Decoder for Dropped {
    fn push_pixel(&mut self, _: bool) {}

    fn push_pixel_chunk(&mut self, _: bool, _: u32) {}

    fn next_line(&mut self) {}
}

/// A decoding filter and its parameters.
#[derive(Clone, Debug)]
pub(crate) struct Filter {
    pub(super) name: Rc<str>,
    /// `SubFileDecode`'s count and end-of-data string.
    pub(super) sub: (usize, Vec<u8>),
    /// `FlateDecode`'s and `LZWDecode`'s `Predictor`: what it undoes.
    pub(super) predictor: Option<Predictor>,
    /// `CCITTFaxDecode`'s parameters.
    pub(super) fax: Option<DecodeSettings>,
}

/// A `Predictor` (2: TIFF, 10–15: PNG) with its `Colors`, `BitsPerComponent` and `Columns`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Predictor {
    kind: u8,
    colors: usize,
    bpc: usize,
    columns: usize,
}

impl Predictor {
    /// The predictor of a filter's parameters, if it has one.
    pub(super) fn of(d: &DictRef) -> Res<Option<Self>> {
        let num = |k: &str, default: f64| d.borrow().get(&Key::name(k)).and_then(Obj::as_num).unwrap_or(default);
        let kind = num("Predictor", 1.0);
        if kind <= 1.0 {
            return Ok(None);
        }
        let (colors, bpc, columns) = (num("Colors", 1.0), num("BitsPerComponent", 8.0), num("Columns", 1.0));
        if !(kind == 2.0 || (10.0..=15.0).contains(&kind))
            || !(1.0..=32.0).contains(&colors)
            || ![1.0, 2.0, 4.0, 8.0, 16.0].contains(&bpc)
            || !(1.0..=1e6).contains(&columns)
        {
            return ps_err("rangecheck", "Predictor");
        }
        Ok(Some(Self { kind: kind as u8, colors: colors as usize, bpc: bpc as usize, columns: columns as usize }))
    }

    /// The data the predictor made `data` from.
    fn undo(self, data: &[u8]) -> Vec<u8> {
        // A row longer than the data is the data.
        let row = (self.colors * self.bpc * self.columns).div_ceil(8).min(data.len().max(1));
        // Bytes per pixel (at least one) is how far back the left neighbour is.
        let bpp = (self.colors * self.bpc).div_ceil(8).max(1);
        if self.kind == 2 {
            // Horizontal differencing (ISO 32000-1, §7.4.4.4): each component is stored as its
            // difference from the same component of the pixel to its left, wrapping within its bit
            // width. Each row starts again, and the padding bits at a row's end stay as they are.
            let mut out = data.to_vec();
            for line in out.chunks_mut(row.max(1)) {
                match self.bpc {
                    8 => {
                        for i in bpp..line.len() {
                            let left = line.get(i - bpp).copied().unwrap_or(0);
                            if let Some(b) = line.get_mut(i) {
                                *b = b.wrapping_add(left);
                            }
                        }
                    }
                    // 16-bit components are big-endian and add as 16-bit numbers.
                    16 => {
                        let byte = |line: &[u8], i: usize| line.get(i).copied().unwrap_or(0);
                        for i in (bpp..line.len().saturating_sub(1)).step_by(2) {
                            let left = u16::from_be_bytes([byte(line, i - bpp), byte(line, i - bpp + 1)]);
                            let sum = u16::from_be_bytes([byte(line, i), byte(line, i + 1)]).wrapping_add(left);
                            if let Some(w) = line.get_mut(i..i + 2) {
                                w.copy_from_slice(&sum.to_be_bytes());
                            }
                        }
                    }
                    // 1, 2 or 4 bits: components are packed from each byte's high bit.
                    bits @ (1 | 2 | 4) => {
                        let mask = (1u8 << bits) - 1;
                        let shift = |k: usize| 8 - bits - k * bits % 8;
                        let sample = |line: &[u8], k: usize| line.get(k * bits / 8).map_or(0, |b| (b >> shift(k)) & mask);
                        let samples = (line.len() * 8 / bits).min(self.colors * self.columns);
                        for k in self.colors..samples {
                            let sum = sample(line, k).wrapping_add(sample(line, k - self.colors)) & mask;
                            if let Some(b) = line.get_mut(k * bits / 8) {
                                *b = (*b & !(mask << shift(k))) | (sum << shift(k));
                            }
                        }
                    }
                    // `Predictor::of` accepts no other depth.
                    _ => {}
                }
            }
            return out;
        }
        // PNG: each row starts with its filter type.
        let mut out: Vec<u8> = Vec::with_capacity(data.len());
        let mut prev = vec![0u8; row];
        for chunk in data.chunks(row + 1) {
            let Some((&kind, line)) = chunk.split_first() else { break };
            let mut cur = line.to_vec();
            cur.resize(row, 0);
            for i in 0..row {
                let a = if i >= bpp { cur.get(i - bpp).copied().unwrap_or(0) } else { 0 };
                let b = prev.get(i).copied().unwrap_or(0);
                let c = if i >= bpp { prev.get(i - bpp).copied().unwrap_or(0) } else { 0 };
                let add = match kind {
                    1 => a,
                    2 => b,
                    3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                    4 => paeth(a, b, c),
                    _ => 0,
                };
                if let Some(v) = cur.get_mut(i) {
                    *v = v.wrapping_add(add);
                }
            }
            out.extend_from_slice(cur.get(..line.len().min(row)).unwrap_or_default());
            prev = cur;
        }
        out
    }
}

/// The PNG Paeth predictor.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i16::from(a), i16::from(b), i16::from(c));
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// `CCITTFaxDecode`'s parameters (PLRM 3rd ed., §3.13, `CCITTFaxDecode` parameters; ISO 32000-2,
/// §7.4.6): `K` (below 0 Group 4, 0 Group 3 one-dimensional, above 0 Group 3 mixed), `Columns`,
/// `Rows`, `EndOfBlock`, `EndOfLine`, `EncodedByteAlign` and `BlackIs1`, with their defaults where
/// `d` leaves them out. `Columns` outside 1 to 65,535 and `Rows` below 0 are a `rangecheck`.
pub(super) fn fax_settings(d: Option<&DictRef>) -> Res<DecodeSettings> {
    let get = |k: &str| d.and_then(|d| d.borrow().get(&Key::name(k)).cloned());
    let num = |k: &str, default: f64| get(k).and_then(|o| o.as_num()).unwrap_or(default);
    let flag = |k: &str, default: bool| match get(k) {
        Some(Obj::Bool(b)) => b,
        _ => default,
    };
    let (k, columns, rows) = (num("K", 0.0), num("Columns", 1728.0), num("Rows", 0.0));
    if !k.is_finite() || !(1.0..=65_535.0).contains(&columns) || !(0.0..=f64::from(u32::MAX)).contains(&rows) {
        return ps_err("rangecheck", "CCITTFaxDecode");
    }
    let encoding = if k < 0.0 {
        EncodingMode::Group4
    } else if k == 0.0 {
        EncodingMode::Group3_1D
    } else {
        EncodingMode::Group3_2D { k: k.min(f64::from(u32::MAX)) as u32 }
    };
    Ok(DecodeSettings {
        columns: columns as u32,
        rows: rows as u32,
        end_of_block: flag("EndOfBlock", true),
        end_of_line: flag("EndOfLine", false),
        rows_are_byte_aligned: flag("EncodedByteAlign", false),
        encoding,
        invert_black: flag("BlackIs1", false),
    })
}

/// The bytes `f` decodes from `raw`, how many it read, and the channels of a decoded JPEG.
pub(super) fn decode(f: &Filter, raw: &[u8]) -> Res<(Vec<u8>, usize, Option<u8>)> {
    let (data, used) = match &*f.name {
        "ASCIIHexDecode" => {
            let end = raw.iter().position(|b| *b == b'>');
            let body = raw.get(..end.unwrap_or(raw.len())).unwrap_or_default();
            (hex_decode(body).ok_or(PsError::Ps("ioerror", "ASCIIHexDecode".into()))?, end.map_or(raw.len(), |e| e + 1))
        }
        "ASCII85Decode" => {
            let end = find(raw, b"~>");
            let mut text = String::from_utf8_lossy(raw.get(..end.unwrap_or(raw.len())).unwrap_or_default()).into_owned();
            text.push_str("~>");
            (ps::ascii85_decode(&text).ok_or(PsError::Ps("ioerror", "ASCII85Decode".into()))?, end.map_or(raw.len(), |e| e + 2))
        }
        "RunLengthDecode" => run_length(raw)?,
        "FlateDecode" | "LZWDecode" => {
            let (data, used) = if &*f.name == "FlateDecode" { flate(raw)? } else { lzw(raw)? };
            match f.predictor {
                Some(p) => (p.undo(&data), used),
                None => (data, used),
            }
        }
        "DCTDecode" => {
            let (d, used, channels) = dct(raw)?;
            return Ok((d, used, Some(channels)));
        }
        "CCITTFaxDecode" => match f.fax {
            Some(settings) => fax(raw, settings)?,
            None => return io("CCITTFaxDecode"),
        },
        "SubFileDecode" => {
            let (count, eod) = &f.sub;
            if eod.is_empty() {
                // A count of 0 without an end string sets no end: the data passes through whole.
                let n = if *count == 0 { raw.len() } else { (*count).min(raw.len()) };
                (raw.get(..n).unwrap_or_default().to_vec(), n)
            } else {
                let (mut at, mut seen) = (0, 0);
                loop {
                    match find(raw.get(at..).unwrap_or_default(), eod) {
                        Some(i) if seen < *count => {
                            seen += 1;
                            at += i + eod.len();
                        }
                        Some(i) => break (raw.get(..at + i).unwrap_or_default().to_vec(), at + i + eod.len()),
                        None => break (raw.to_vec(), raw.len()),
                    }
                }
            }
        }
        // Reads all of its source (up to the end of the filters below it) and keeps it.
        "NullEncode" | "ReusableStreamDecode" => (raw.to_vec(), raw.len()),
        other => return ps_err("undefined", other),
    };
    Ok((data, used, None))
}
