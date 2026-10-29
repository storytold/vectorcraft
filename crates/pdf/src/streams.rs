//! Streams looked at before the PDF reader (hayro) decodes them. It inflates Flate and LZW data
//! with no ceiling, so a few megabytes can ask for gigabytes (#1700), and it applies a predictor
//! before checking its parameters: `Columns 0` divides by zero, a huge `Columns` allocates a row
//! that large whatever the stream holds (#1701). A file with such a stream is refused instead.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::Read;

use craft_pdf::syntax::content::TypedIter;
use craft_pdf::syntax::content::ops::TypedInstruction;
use craft_pdf::syntax::object::{Array, Dict, FromBytes, MaybeRef, Name, Object, Stream};

/// The most a stream may decode to: 256 MB, the pixels of the largest image the app reads (64
/// million, 4 bytes each).
pub(crate) const MAX_DECODED: usize = 256 << 20;

/// The most an image XObject may decode to when its own size asks for more than
/// [`MAX_DECODED`]: 1 GB (an A0 sheet scanned at 300 ppi is 418 MB in RGB, 557 MB in CMYK).
const MAX_IMAGE_DECODED: usize = 1 << 30;

/// The longest row a predictor may have, in bytes.
const MAX_ROW: usize = 1 << 24;

/// A stream that decodes to more than its [`budget`].
struct TooBig;

/// The filters of the stream with dictionary `dict`, by name (`Filter` or `F`: a name or an array).
fn filters<'a>(dict: &Dict<'a>) -> Vec<Name<'a>> {
    [&b"Filter"[..], b"F"]
        .into_iter()
        .find_map(|key| dict.get::<Name<'a>>(key).map(|n| vec![n]).or_else(|| dict.get::<Array<'a>>(key).map(|a| a.iter::<Name<'a>>().collect())))
        .unwrap_or_default()
}

/// Every decode parameter dictionary of the stream with dictionary `dict`, as the reader may take
/// them (`DecodeParms` or `DP`: a dictionary, or an array with one for each filter).
fn params<'a>(dict: &Dict<'a>) -> Vec<Dict<'a>> {
    let mut all = vec![];
    for key in [&b"DecodeParms"[..], b"DP"] {
        all.extend(dict.get::<Dict<'a>>(key));
        all.extend(dict.get::<Array<'a>>(key).into_iter().flat_map(|a| a.iter::<Object<'a>>().filter_map(|o| o.into_dict()).collect::<Vec<_>>()));
    }
    all
}

/// Would the reader's predictor fail on `params`? It reads `Colors` and `BitsPerComponent` as
/// bytes and `Columns` as a size, multiplies the first two as bytes, divides by the length of a row
/// and allocates one before it checks anything.
fn bad_predictor(params: &Dict<'_>) -> bool {
    if params.get::<u8>(b"Predictor").unwrap_or(1) == 1 {
        return false;
    }
    let colors = usize::from(params.get::<u8>(b"Colors").unwrap_or(1));
    let bits = usize::from(params.get::<u8>(b"BitsPerComponent").unwrap_or(8));
    let columns = params.get::<usize>(b"Columns").unwrap_or(1);
    let pixel = colors * bits;
    let row = columns.checked_mul(pixel).map(|b| b.div_ceil(8));
    !(matches!(bits, 1 | 2 | 4 | 8 | 16) && colors > 0 && pixel <= 255 && columns > 0 && row.is_some_and(|r| r <= MAX_ROW))
}

/// `data` inflated, up to one byte past `max` (zlib, or raw deflate as the reader also takes);
/// `None` when it is damaged before that.
fn inflate(data: &[u8], max: usize) -> Option<Vec<u8>> {
    let read = |mut r: Box<dyn Read + '_>| {
        let mut out = vec![];
        // (Damaged data: what came out before the damage still counts.)
        let done = r.by_ref().take(max as u64 + 1).read_to_end(&mut out).is_ok();
        (done || out.len() > max).then_some(out)
    };
    read(Box::new(flate2::read::ZlibDecoder::new(data))).or_else(|| read(Box::new(flate2::read::DeflateDecoder::new(data))))
}

/// `data` decoded from LZW, up to one byte past `max`. `early`: the code length grows one code
/// early (`EarlyChange`, 1 by default).
fn lzw(data: &[u8], early: bool, max: usize) -> Option<Vec<u8>> {
    use weezl::{BitOrder, LzwStatus, decode::Decoder};
    let mut decoder = if early { Decoder::with_tiff_size_switch(BitOrder::Msb, 8) } else { Decoder::new(BitOrder::Msb, 8) };
    let (mut out, mut chunk, mut input) = (vec![], vec![0; 1 << 16], data);
    while out.len() <= max {
        let r = decoder.decode_bytes(input, &mut chunk);
        input = input.get(r.consumed_in..).unwrap_or_default();
        out.extend_from_slice(chunk.get(..r.consumed_out).unwrap_or_default());
        match r.status {
            Ok(LzwStatus::Ok) => {}
            Ok(LzwStatus::Done | LzwStatus::NoProgress) => break,
            Err(_) => return (out.len() > max).then_some(out),
        }
    }
    Some(out)
}

/// `data` decoded from run-length encoding, up to one byte past `max`.
fn run_length(data: &[u8], max: usize) -> Vec<u8> {
    let (mut out, mut at) = (vec![], 0);
    while let Some(&n) = data.get(at)
        && n != 128
        && out.len() <= max
    {
        if n < 128 {
            let take = usize::from(n) + 1;
            out.extend_from_slice(data.get(at + 1..).unwrap_or_default().get(..take).unwrap_or_default());
            at += 1 + take;
        } else {
            out.extend(std::iter::repeat_n(data.get(at + 1).copied().unwrap_or(0), 257 - usize::from(n)));
            at += 2;
        }
    }
    out
}

/// `data` decoded from hexadecimal digits (white space skipped, up to `>`).
fn hex(data: &[u8]) -> Vec<u8> {
    let digits: Vec<u8> = data.iter().take_while(|c| **c != b'>').filter_map(|c| char::from(*c).to_digit(16)).map(|d| d as u8).collect();
    digits.chunks(2).map(|p| (p.first().copied().unwrap_or(0) << 4) | p.get(1).copied().unwrap_or(0)).collect()
}

/// `data` decoded from base-85 (white space skipped, `z` for four zeros, up to `~>`).
fn base85(data: &[u8]) -> Vec<u8> {
    let (mut out, mut group) = (vec![], vec![]);
    let flush = |group: &mut Vec<u32>, out: &mut Vec<u8>| {
        let n = group.len();
        if n > 1 {
            group.resize(5, 84);
            let v = group.iter().fold(0u32, |v, d| v.wrapping_mul(85).wrapping_add(*d));
            out.extend_from_slice(v.to_be_bytes().get(..n - 1).unwrap_or_default());
        }
        group.clear();
    };
    for &c in data.iter().take_while(|c| **c != b'~') {
        match c {
            b'z' if group.is_empty() => out.extend([0; 4]),
            b'!'..=b'u' => {
                group.push(u32::from(c - b'!'));
                if group.len() == 5 {
                    flush(&mut group, &mut out);
                }
            }
            _ => {}
        }
    }
    flush(&mut group, &mut out);
    out
}

/// The most data of `len` bytes can grow to through `filters`: 1,032 times through Flate (its
/// largest ratio), 3,641 through LZW (4,096 bytes from a 9-bit code), 64 through run-length.
fn most_after(len: usize, filters: &[Name<'_>]) -> u128 {
    filters.iter().fold(len as u128, |n, f| {
        n.saturating_mul(match f.as_ref() {
            b"FlateDecode" | b"Fl" => 1032,
            b"LZWDecode" | b"LZW" => 3641,
            b"RunLengthDecode" | b"RL" => 64,
            _ => 1,
        })
    })
}

/// The number of colour components of the image with dictionary `dict`, from its colour space
/// (4 when it can't be told).
fn components(dict: &Dict<'_>) -> u64 {
    let named = |n: &[u8]| match n {
        b"DeviceGray" | b"G" | b"CalGray" | b"Indexed" | b"I" | b"Separation" => 1,
        b"DeviceRGB" | b"RGB" | b"CalRGB" | b"Lab" => 3,
        _ => 4,
    };
    if let Some(n) = dict.get::<Name<'_>>(b"ColorSpace") {
        return named(n.as_ref());
    }
    let Some(a) = dict.get::<Array<'_>>(b"ColorSpace") else { return 4 };
    let mut it = a.flex_iter();
    let any = |n: Option<usize>| n.filter(|n| (1..=32).contains(n)).map_or(4, |n| n as u64);
    match it.next::<Name<'_>>() {
        Some(n) if n.as_ref() == b"ICCBased" => any(it.next::<Stream<'_>>().and_then(|s| s.dict().get::<usize>(b"N"))),
        Some(n) if n.as_ref() == b"DeviceN" => any(it.next::<Array<'_>>().map(|names| names.iter::<Object<'_>>().count())),
        Some(n) => named(n.as_ref()),
        None => 4,
    }
}

/// The most the stream with dictionary `dict` may decode to: [`MAX_DECODED`], or for an image
/// XObject the size its width, height, components and bits declare (each row with a predictor's
/// tag byte), up to [`MAX_IMAGE_DECODED`]: an image's size is known before it is decoded.
fn budget(dict: &Dict<'_>) -> usize {
    if dict.get::<Name<'_>>(b"Subtype").is_none_or(|s| s.as_ref() != b"Image") {
        return MAX_DECODED;
    }
    let side = |key: &[u8]| dict.get::<u32>(key).map_or(0, u64::from);
    let mask = dict.get::<bool>(b"ImageMask") == Some(true);
    let bits = if mask { 1 } else { dict.get::<u8>(b"BitsPerComponent").map_or(8, |b| u64::from(b.min(16))) };
    let channels = if mask { 1 } else { components(dict) };
    let row = side(b"Width").saturating_mul(channels).saturating_mul(bits).div_ceil(8).saturating_add(1);
    let declared = usize::try_from(side(b"Height").saturating_mul(row)).unwrap_or(usize::MAX);
    declared.clamp(MAX_DECODED, MAX_IMAGE_DECODED)
}

/// `data` (of the stream with dictionary `dict`) through its filters, as far as this reads them:
/// Flate, LZW, run-length and the ASCII encodings. Not decoded when it can't decode past its
/// [`budget`] and `keep` doesn't ask for it, or when an image codec or damaged data comes first
/// (an image's own decoder knows its size).
fn decode(data: &[u8], dict: &Dict<'_>, keep: bool) -> Result<Option<Vec<u8>>, TooBig> {
    let filters = filters(dict);
    let max = budget(dict);
    if !keep && most_after(data.len(), &filters) <= max as u128 {
        return Ok(None);
    }
    let params = params(dict);
    let mut data = Cow::Borrowed(data);
    for (i, f) in filters.iter().enumerate() {
        let early = params.get(i).and_then(|p| p.get::<u8>(b"EarlyChange")).is_none_or(|e| e != 0);
        let out = match f.as_ref() {
            b"FlateDecode" | b"Fl" => inflate(&data, max),
            b"LZWDecode" | b"LZW" => lzw(&data, early, max),
            b"RunLengthDecode" | b"RL" => Some(run_length(&data, max)),
            b"ASCIIHexDecode" | b"AHx" => Some(hex(&data)),
            b"ASCII85Decode" | b"A85" => Some(base85(&data)),
            _ => None,
        };
        match out {
            Some(out) if out.len() > max => return Err(TooBig),
            Some(out) => data = Cow::Owned(out),
            None => return Ok(None),
        }
    }
    Ok(Some(data.into_owned()))
}

/// May the stream with dictionary `dict` be content (of a page, form, pattern or glyph), which can
/// hold inline images? Not an image, font, profile, function, metadata or object stream.
fn maybe_content(dict: &Dict<'_>) -> bool {
    let named = |key: &[u8], ok: &[&[u8]]| dict.get::<Name<'_>>(key).is_none_or(|n| ok.contains(&n.as_ref()));
    named(b"Subtype", &[b"Form"])
        && named(b"Type", &[b"XObject", b"Pattern"])
        && ![&b"Length1"[..], b"Length2", b"Length3", b"FunctionType", b"N"].iter().any(|k| dict.contains_key(k))
}

/// Would the reader fail on `stream` (or on an inline image in it, when it is content): a
/// predictor it can't apply, or more than its [`budget`] decoded?
pub(crate) fn unsafe_stream(stream: &Stream<'_>) -> bool {
    unsafe_data(stream, true)
}

/// [`unsafe_stream`], looking for inline images in it when `inside` (not in an inline image's own
/// data: it would be read again for each inline image a hostile one nests).
fn unsafe_data(stream: &Stream<'_>, inside: bool) -> bool {
    let dict = stream.dict();
    if params(dict).iter().any(bad_predictor) {
        return true;
    }
    let content = inside && maybe_content(dict);
    match decode(&stream.raw_data(), dict, content) {
        Err(TooBig) => true,
        Ok(Some(data)) if content && data.windows(2).any(|w| w == b"ID") => {
            let mut ops = TypedIter::new(&data);
            while let Some(op) = ops.next() {
                if let TypedInstruction::InlineImage(image) = op
                    && unsafe_data(image.0, false)
                {
                    return true;
                }
            }
            false
        }
        Ok(_) => false,
    }
}

/// Would the reader fail on a stream it decodes while it opens `pdf`, before anything else can
/// look at it: an object stream or a cross-reference stream (read here from the file's text)?
/// Their decode parameters must be in their dictionary, as the reader can't be asked for others
/// yet. Encrypted object streams are looked at once the file is open (their data is ciphered
/// here).
///
/// The file is read once from the start, as the reader reads PDF: literal strings (nested
/// parentheses, `\` escaping), hex strings and comments are skipped, `<<` and `>>` pair up, and a
/// dictionary that `stream` follows (white space and comments between) is a stream's, whose data
/// is skipped. So a `stream` or `>>` in a string or a comment is no keyword (#3919 review). The
/// reader may still start at a `<<` inside what this reading skipped (a cross-reference offset or
/// its repair scan can point there), so each such `<<` it could start at is read as a dictionary
/// of its own, within a budget for the whole file: a file that spends it is refused. Linear in
/// the file (#3586), a run of `<` included (#3924 review).
pub(crate) fn unsafe_file_streams(pdf: &[u8]) -> bool {
    let mut scan = FileScan {
        pdf,
        budget: pdf.len().saturating_mul(4).saturating_add(1 << 20),
        endstream: None,
        lines: BTreeMap::new(),
        strings: None,
        read_from: Vec::new(),
    };
    scan.read().is_err()
}

/// A stream the reader would fail on, or a file that spent the scan's budget.
struct Unsafe;

/// The most bytes a dictionary read from inside a string, comment or stream data may span past
/// the last `<<` in it, as far as the scan looked back from a `stream` keyword before.
const MAX_SKIPPED_DICT: usize = 1 << 16;

/// The most `<<` the scan keeps open: deeper than the reader nests dictionaries.
const MAX_OPEN: usize = 1024;

/// PDF white space (ISO 32000-1, 7.2.2).
fn white(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

/// Where `what` is first found in `pdf` at or after `from`.
fn find(pdf: &[u8], from: usize, what: &[u8]) -> Option<usize> {
    pdf.get(from..).and_then(|rest| rest.windows(what.len()).position(|w| w == what)).map(|at| from + at)
}

/// Past the literal string opening at `at` (a `(`), as the reader reads it: parentheses nest and
/// `\` escapes the byte after it. `Err` with how far it got when it doesn't close before `limit`.
fn string_end(pdf: &[u8], at: usize, limit: usize) -> Result<usize, usize> {
    let (mut depth, mut i) = (0usize, at);
    while let Some(&b) = pdf.get(i).filter(|_| i < limit) {
        match b {
            b'\\' => i += 1,
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err(i.min(limit))
}

/// How far past its `(` a string read in skipped text may end to be read again each time it is
/// entered (bytes); one ending further is looked up in [`StringEnds`].
const NEAR_STRING: usize = 64;

/// Is the byte at `at` escaped, a `\` before it that isn't escaped itself? Reading a string from
/// any `(` before it sees the same, as the run of `\` before it lies after that `(`.
fn escaped(pdf: &[u8], at: usize) -> bool {
    pdf.get(..at).unwrap_or_default().iter().rev().take_while(|b| **b == b'\\').count() % 2 == 1
}

/// Where the literal strings read from the file's `(` end, for those ending more than
/// [`NEAR_STRING`] bytes on (#3924 review). As whether a byte is escaped doesn't depend on where
/// reading started ([`escaped`]), the string from any `(` ends after the first `)` that takes the
/// count of unescaped `(` less unescaped `)` from the file's start below the count with that `(`
/// (counted even when escaped). So all are found in three passes over the file: forward for the
/// count at its end, back for the `(` whose string never ends (the count never falls below theirs
/// again), and forward pairing the others with a stack.
struct StringEnds {
    /// Each `(` whose string ends more than [`NEAR_STRING`] bytes on, and where, in file order.
    far: Vec<(usize, usize)>,
}

impl StringEnds {
    /// The ends in `pdf`; `None` when more than one in 16 bytes of it are kept or pending (only a
    /// file built for it nests strings so deep).
    fn of(pdf: &[u8]) -> Option<StringEnds> {
        let most = pdf.len() / 16 + 1024;
        // What the byte at `i` adds to the count: an escaped `(` adds nothing but is one too.
        let step = |i: usize| match pdf.get(i) {
            Some(b'(') => Some(i64::from(!escaped(pdf, i))),
            Some(b')') => (!escaped(pdf, i)).then_some(-1),
            _ => None,
        };
        let total: i64 = (0..pdf.len()).filter_map(step).sum();
        // The `(` whose string never ends: none after it takes the count below its own.
        let mut endless = vec![0u64; pdf.len() / 64 + 1];
        let (mut count, mut lowest) = (total, i64::MAX);
        for i in (0..pdf.len()).rev() {
            let Some(d) = step(i) else { continue };
            if pdf.get(i) == Some(&b'(')
                && count <= lowest
                && let Some(word) = endless.get_mut(i / 64)
            {
                *word |= 1 << (i % 64);
            }
            lowest = lowest.min(count);
            count -= d;
        }
        let (mut far, mut open, mut count) = (vec![], Vec::<(usize, i64)>::new(), 0);
        for i in 0..pdf.len() {
            let Some(d) = step(i) else { continue };
            count += d;
            if pdf.get(i) == Some(&b'(') {
                if endless.get(i / 64).is_some_and(|word| word >> (i % 64) & 1 == 0) {
                    open.push((i, count));
                }
            } else {
                while let Some(&(at, _)) = open.last().filter(|(_, level)| *level > count) {
                    open.pop();
                    if i + 1 - at > NEAR_STRING {
                        far.push((at, i + 1));
                    }
                }
            }
            if far.len() + open.len() > most {
                return None;
            }
        }
        far.sort_unstable();
        Some(StringEnds { far })
    }

    /// Where the string from the `(` at `at` ends, if it ends more than [`NEAR_STRING`] bytes on;
    /// `None` for one that doesn't end.
    fn far(&self, at: usize) -> Option<usize> {
        self.far.binary_search_by_key(&at, |(open, _)| *open).ok().and_then(|n| self.far.get(n)).map(|(_, end)| *end)
    }
}

/// Past the hex string opening at `at` (a `<`): hex digits and white space, then `>`. `Err` with
/// how far it got for one the reader fails on.
fn hex_end(pdf: &[u8], at: usize, limit: usize) -> Result<usize, usize> {
    let mut i = at + 1;
    while pdf.get(i).is_some_and(|b| i < limit && (b.is_ascii_hexdigit() || white(*b))) {
        i += 1;
    }
    if i < limit && pdf.get(i) == Some(&b'>') { Ok(i + 1) } else { Err(i.min(limit)) }
}

/// The end of the comment starting at `at` (a `%`): its end of line, not included.
fn comment_end(pdf: &[u8], at: usize) -> usize {
    pdf.get(at..).and_then(|rest| rest.iter().position(|b| matches!(b, b'\n' | b'\r'))).map_or(pdf.len(), |n| at + n)
}

/// Can the reader start a dictionary at the `<<` at `at`? Its repair scan tries one at the start
/// of the file, after white space and after a dictionary it read (`>>`); an object that a
/// cross-reference offset points at begins `N G obj`, then white space or comments. A `<<` after
/// anything else is read only as part of what is before it: in a run of `<`, only the first.
fn startable(pdf: &[u8], at: usize) -> bool {
    let before = pdf.get(..at).unwrap_or_default();
    before.last().is_none_or(|b| white(*b)) || before.ends_with(b">>") || before.ends_with(b"obj")
}

/// The pre-open scan of a file's streams ([`unsafe_file_streams`]).
struct FileScan<'a> {
    pdf: &'a [u8],
    /// The bytes left to read again: dictionaries inside skipped text, `endstream` searches and
    /// the data decoded. Every byte of a well-formed file is read about once.
    budget: usize,
    /// Where an `endstream` was last looked for, and the first one found there (`None`: none).
    endstream: Option<(usize, Option<usize>)>,
    /// The comments read in skipped text: each one's end of line, and where reading it began.
    lines: BTreeMap<usize, usize>,
    /// The ends of strings read in skipped text far from their `(` ([`StringEnds`]), found once
    /// the first such string is read.
    strings: Option<StringEnds>,
    /// The `<<` a dictionary in skipped text was read from, a bit a byte of the file (empty
    /// until one is).
    read_from: Vec<u64>,
}

impl FileScan<'_> {
    fn charge(&mut self, bytes: usize) -> Result<(), Unsafe> {
        self.budget = self.budget.checked_sub(bytes).ok_or(Unsafe)?;
        Ok(())
    }

    /// The file from the start, as the reader reads it.
    fn read(&mut self) -> Result<(), Unsafe> {
        let pdf = self.pdf;
        let mut opens = std::collections::VecDeque::new();
        // The dictionary just read, while only white space and comments follow it.
        let mut closed: Option<(usize, usize)> = None;
        let mut i = 0;
        while let Some(&b) = pdf.get(i) {
            if white(b) {
                i += 1;
                continue;
            }
            if b == b'%' {
                let end = comment_end(pdf, i);
                self.skipped(i, end)?;
                i = end;
                continue;
            }
            let last = closed.take();
            match pdf.get(i..i + 2) {
                Some(b"<<") => {
                    if opens.len() == MAX_OPEN {
                        opens.pop_front();
                    }
                    opens.push_back(i);
                    i += 2;
                }
                Some(b">>") => {
                    closed = opens.pop_back().map(|open| (open, i + 2));
                    i += 2;
                }
                _ if b == b'(' => {
                    let end = string_end(pdf, i, pdf.len()).unwrap_or(pdf.len());
                    self.skipped(i, end)?;
                    i = end;
                }
                _ if b == b'<' => i = hex_end(pdf, i, pdf.len()).unwrap_or(i + 1),
                _ => match last {
                    Some((open, close)) if pdf.get(i..).is_some_and(|rest| rest.starts_with(b"stream")) => {
                        let end = self.stream(open, close, i + 6)?;
                        self.skipped(i + 6, end)?;
                        i = end.max(i + 6);
                    }
                    _ => i += 1,
                },
            }
        }
        Ok(())
    }

    /// Each `<<` in `from..to`, text the reading skipped, that the reader could start at, read as
    /// a dictionary of its own.
    fn skipped(&mut self, from: usize, to: usize) -> Result<(), Unsafe> {
        let text = self.pdf.get(..to).unwrap_or_default();
        let mut i = from;
        while let Some(at) = find(text, i, b"<<") {
            if startable(self.pdf, at) {
                self.dicts(at)?;
            }
            i = at + 1;
        }
        Ok(())
    }

    /// The dictionary at `at` in skipped text, read to the `>>` that closes it, as far as
    /// [`MAX_SKIPPED_DICT`] past the last `<<` opened on the way; and each dictionary it holds.
    /// Each `>>` closing one is looked at for a stream after it, and each `<<` opening one is
    /// marked: a reading from there reads the same text as far, so a later call for it returns
    /// at once and text dense with `<<` is read about once.
    fn dicts(&mut self, at: usize) -> Result<(), Unsafe> {
        if self.was_read(at) {
            return Ok(());
        }
        let pdf = self.pdf;
        let mut opens = std::collections::VecDeque::new();
        let (mut depth, mut limit, mut i, mut work) = (0usize, at.saturating_add(MAX_SKIPPED_DICT), at, 0usize);
        while let Some(&b) = pdf.get(i).filter(|_| i < limit) {
            // Where to read on (`Err`: the dictionary ends there, read or unreadable), and the
            // bytes read to get there.
            let (next, read) = match pdf.get(i..i + 2) {
                Some(b"<<") => {
                    self.mark(i);
                    limit = limit.max(i.saturating_add(MAX_SKIPPED_DICT));
                    if opens.len() == MAX_OPEN {
                        opens.pop_front();
                    }
                    opens.push_back(i);
                    depth += 1;
                    (Ok(i + 2), 2)
                }
                Some(b">>") => {
                    depth = depth.saturating_sub(1);
                    if let Some(open) = opens.pop_back() {
                        self.closed(open, i + 2)?;
                    }
                    (if depth == 0 { Err(i + 2) } else { Ok(i + 2) }, 2)
                }
                _ => match b {
                    // Strings and comments are charged as they are first read.
                    b'(' => (self.string_end(i)?.ok_or(pdf.len()), 0),
                    b'%' => (Ok(self.line_end(i)?), 0),
                    b'<' => {
                        let end = hex_end(pdf, i, limit);
                        let (Ok(n) | Err(n)) = end;
                        (end, n.saturating_sub(i))
                    }
                    _ => (Ok(i + 1), 1),
                },
            };
            work += read;
            let Ok(n) = next else { break };
            i = n;
        }
        self.charge(work)
    }

    /// The dictionary `open..close` in skipped text: the stream it begins if `stream` follows it.
    fn closed(&mut self, open: usize, close: usize) -> Result<(), Unsafe> {
        let mut keyword = close;
        while let Some(&b) = self.pdf.get(keyword) {
            match b {
                b'%' => keyword = self.line_end(keyword)?,
                b if white(b) => keyword += 1,
                _ => break,
            }
        }
        self.charge(keyword - close)?;
        if self.pdf.get(keyword..).is_some_and(|rest| rest.starts_with(b"stream")) {
            self.stream(open, close, keyword + 6)?;
        }
        Ok(())
    }

    /// The end of the comment at `at` ([`comment_end`]). Comments in skipped text are read from
    /// many places, so each one's end is kept: a byte is read as comment once.
    fn line_end(&mut self, at: usize) -> Result<usize, Unsafe> {
        // The first comment kept that ends at or after `at`.
        let kept = self.lines.range(at..).next().map(|(end, from)| (*end, *from));
        if let Some((end, from)) = kept
            && from <= at
        {
            return Ok(end);
        }
        // Read up to the next end of line, or to where that comment was read from.
        let upto = kept.map_or(self.pdf.len(), |(_, from)| from);
        let rest = self.pdf.get(at..upto).unwrap_or_default();
        let end = match rest.iter().position(|b| matches!(b, b'\n' | b'\r')) {
            Some(n) => at + n,
            None => kept.map_or(self.pdf.len(), |(end, _)| end),
        };
        self.charge(rest.len().min(end - at))?;
        self.lines.insert(end, at);
        Ok(end)
    }

    /// Past the literal string at `at` (a `(`), as the reader reads it; `None` for one that
    /// doesn't end. A string ending within [`NEAR_STRING`] bytes of its `(` is read again each
    /// time a reading enters it; one ending further is looked up in [`StringEnds`], found for the
    /// whole file in three passes (linear, so not charged) the first time one is needed. So each
    /// time a string is entered costs at most [`NEAR_STRING`] bytes, charged as one.
    fn string_end(&mut self, at: usize) -> Result<Option<usize>, Unsafe> {
        self.charge(1)?;
        if let Ok(end) = string_end(self.pdf, at, at.saturating_add(NEAR_STRING)) {
            return Ok(Some(end));
        }
        if self.strings.is_none() {
            self.strings = Some(StringEnds::of(self.pdf).ok_or(Unsafe)?);
        }
        Ok(self.strings.as_ref().and_then(|s| s.far(at)))
    }

    fn mark(&mut self, at: usize) {
        if self.read_from.is_empty() {
            self.read_from = vec![0; self.pdf.len() / 64 + 1];
        }
        if let Some(word) = self.read_from.get_mut(at / 64) {
            *word |= 1 << (at % 64);
        }
    }

    fn was_read(&self, at: usize) -> bool {
        self.read_from.get(at / 64).is_some_and(|word| word >> (at % 64) & 1 == 1)
    }

    /// The stream whose dictionary is `open..close` and whose `stream` keyword ends at `from`:
    /// `Err` when the reader would fail on it, else where its data ends.
    fn stream(&mut self, open: usize, close: usize, from: usize) -> Result<usize, Unsafe> {
        let pdf = self.pdf;
        let cr = usize::from(pdf.get(from) == Some(&b'\r'));
        let start = from + cr + usize::from(pdf.get(from + cr) == Some(&b'\n'));
        self.charge(close.saturating_sub(open))?;
        let dict = pdf.get(open..close).and_then(Dict::from_bytes);
        let length = dict.as_ref().and_then(|d| d.get::<usize>(b"Length")).and_then(|n| start.checked_add(n)).filter(|e| *e <= pdf.len());
        let end = match length {
            Some(end) => end,
            None => self.endstream(start)?,
        };
        let Some(dict) = dict.filter(|d| d.get::<Name<'_>>(b"Type").is_some_and(|t| matches!(t.as_ref(), b"ObjStm" | b"XRef"))) else {
            return Ok(end);
        };
        let indirect = [&b"DecodeParms"[..], b"DP"].iter().any(|k| matches!(dict.get_raw::<Object<'_>>(k), Some(MaybeRef::Ref(_))));
        if indirect || params(&dict).iter().any(bad_predictor) {
            return Err(Unsafe);
        }
        if !filters(&dict).is_empty() {
            self.charge(end.saturating_sub(start))?;
        }
        if matches!(decode(pdf.get(start..end).unwrap_or_default(), &dict, false), Err(TooBig)) {
            return Err(Unsafe);
        }
        Ok(end)
    }

    /// Where the first `endstream` at or after `start` is, or the end of the file.
    fn endstream(&mut self, start: usize) -> Result<usize, Unsafe> {
        let len = self.pdf.len();
        if let Some((from, found)) = self.endstream
            && from <= start
            && found.is_none_or(|f| f >= start)
        {
            return Ok(found.unwrap_or(len));
        }
        let found = find(self.pdf, start, b"endstream");
        self.charge(found.unwrap_or(len).saturating_sub(start))?;
        self.endstream = Some((start, found));
        Ok(found.unwrap_or(len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(text: &str) -> Dict<'_> {
        Dict::from_bytes(text.as_bytes()).unwrap()
    }

    #[test]
    fn predictors_the_reader_can_apply_pass() {
        for ok in [
            "<< >>",
            "<< /Predictor 1 /Columns 0 >>",
            "<< /Predictor 12 /Columns 5 >>",
            "<< /Predictor 2 /Colors 3 /BitsPerComponent 16 /Columns 1000 >>",
        ] {
            assert!(!bad_predictor(&dict(ok)), "{ok}");
        }
        for bad in [
            "<< /Predictor 2 /Columns 0 >>",
            "<< /Predictor 12 /Colors 0 >>",
            "<< /Predictor 2 /BitsPerComponent 0 >>",
            "<< /Predictor 2 /BitsPerComponent 3 >>",
            "<< /Predictor 12 /Colors 32 /BitsPerComponent 16 >>",
            "<< /Predictor 15 /Columns 99999999999 >>",
            "<< /Predictor 10 /Columns 18446744073709551615 /Colors 4 >>",
        ] {
            assert!(bad_predictor(&dict(bad)), "{bad}");
        }
    }

    #[test]
    fn the_decoders_read_what_the_specification_writes() {
        assert_eq!(hex(b"48 65 6c6C6\n>junk"), b"Hell`");
        assert_eq!(base85(b"87cURD]i,\"Ebo80~>"), b"Hello World!");
        assert_eq!(base85(b"z!!~>"), [0, 0, 0, 0, 0]);
        assert_eq!(run_length(&[2, b'a', b'b', b'c', 254, b'x', 128, b'q'], MAX_DECODED), b"abcxxx");
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut e, &[7; 1000]).unwrap();
        assert_eq!(inflate(&e.finish().unwrap(), MAX_DECODED).unwrap(), [7; 1000]);
        // "-----A---B" in LZW, as the specification's example writes it (7.4.4.2).
        let lzw_example = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&lzw_example, true, MAX_DECODED).unwrap(), b"-----A---B");
    }

    /// Zeros past the cap, compressed.
    fn zeros_past_the_cap() -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        for _ in 0..(MAX_DECODED >> 20) + 1 {
            std::io::Write::write_all(&mut e, &[0; 1 << 20]).unwrap();
        }
        e.finish().unwrap()
    }

    #[test]
    fn a_stream_that_decodes_past_the_cap_is_found_without_holding_it() {
        // Inflated in pieces: never more than one byte past the cap is held.
        let bomb = zeros_past_the_cap();
        assert!(bomb.len() < MAX_DECODED / 500);
        assert_eq!(inflate(&bomb, MAX_DECODED).map(|d| d.len()), Some(MAX_DECODED + 1));
        let d = dict("<< /Filter /FlateDecode >>");
        assert!(matches!(decode(&bomb, &d, false), Err(TooBig)));
        // Small enough that it can't get there: not decoded at all.
        assert!(matches!(decode(&[1, 2, 3], &d, false), Ok(None)));
        // An image may decode to its declared size, and no further.
        let image =
            |w: u32, h: u32| format!("<< /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode >>");
        assert!(decode(&bomb, &dict(&image(1 << 15, 1 << 14)), false).is_ok());
        assert!(matches!(decode(&bomb, &dict(&image(16, 16)), false), Err(TooBig)));
        assert_eq!(budget(&dict(&image(1 << 20, 1 << 20))), MAX_IMAGE_DECODED);
    }

    /// A one-page PDF drawing one flat 8-bit DeviceRGB image of `w` × `h` pixels, Flate-compressed.
    fn one_image_pdf(w: usize, h: usize) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        let row: Vec<u8> = [200, 180, 150].repeat(w);
        for _ in 0..h {
            std::io::Write::write_all(&mut e, &row).unwrap();
        }
        let image = e.finish().unwrap();
        let content = b"q 595 0 0 842 0 0 cm /Im0 Do Q";
        let mut out = b"%PDF-1.6\n".to_vec();
        let mut offsets = vec![];
        let objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R >> >> >>".to_vec(),
            [format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(), content, b"\nendstream"].concat(),
            [
                format!(
                    "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
                    image.len()
                )
                .as_bytes(),
                &image,
                b"\nendstream",
            ]
            .concat(),
        ];
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    #[test]
    fn an_image_past_the_stream_cap_within_its_declared_size_opens() {
        // An A0 sheet scanned at 300 ppi: 9930 × 14040 RGB pixels, 418 MB decoded (#2005).
        let pdf = one_image_pdf(9930, 14040);
        let info = crate::info(&pdf, None);
        assert!(info.is_ok(), "{:?}", info.err());
        assert_eq!(info.map(|i| i.pages.len()).unwrap(), 1);
    }

    #[test]
    fn object_and_cross_reference_streams_are_looked_at_before_the_file_is_read() {
        let stream = |dict: &str, data: &[u8]| {
            [format!("1 0 obj\n<< {dict} /Length {} >>\nstream\n", data.len()).as_bytes(), data, b"\nendstream\nendobj\n"].concat()
        };
        for bad in [
            stream("/Type /ObjStm /N 1 /First 4 /Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 0 >>", b"x"),
            stream("/Type /XRef /Filter /FlateDecode /DecodeParms 9 0 R", b"x"),
            stream("/Type /ObjStm /N 1 /First 4 /Filter /FlateDecode", &zeros_past_the_cap()),
        ] {
            assert!(unsafe_file_streams(&bad));
        }
        for fine in [
            stream("/Type /XRef /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 5 >>", b"x"),
            // Another kind of stream is looked at once the file is open, its references followed.
            stream("/Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 0 >>", b"x"),
        ] {
            assert!(!unsafe_file_streams(&fine));
        }
        // A bad stream after others is still found.
        let fine = stream("/Type /XRef /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 5 >>", b"x");
        let bad = stream("/Type /XRef /Filter /FlateDecode /DecodeParms 9 0 R", b"x");
        assert!(unsafe_file_streams(&[fine.as_slice(), b"<< >>stream x endstream", &bad].concat()));
    }

    #[test]
    fn a_file_dense_with_stream_keywords_is_scanned_in_linear_time() {
        // #3586: each `stream` keyword walked back up to 64 KiB, through every dictionary before
        // it, so 1 MiB of keywords took minutes. Now the whole scan takes milliseconds.
        for unit in [&b"<</A <</B 1>> >>stream"[..], b">>stream", b"<</Type/XRef>>stream"] {
            let pdf = unit.repeat((1 << 20) / unit.len());
            let t = std::time::Instant::now();
            assert!(!unsafe_file_streams(&pdf));
            let took = t.elapsed();
            assert!(took < std::time::Duration::from_secs(5), "{} took {took:?}", String::from_utf8_lossy(unit));
        }
    }

    #[test]
    fn a_stream_keyword_in_a_string_or_comment_hides_no_stream_after_it() {
        // #3919 review: a `>>stream` in a literal string (one with a `<<` before it, nested
        // parentheses, an escaped one) or in a comment, a `>>` in a string and a comment between
        // `>>` and `stream` each left the cross-reference stream they were in or before unread.
        let cases = [
            &b"<< /Type /XRef /Note (<<) /Pad (>>stream) /DecodeParms 9 0 R >> stream\nx\nendstream"[..],
            b"<< /Type /XRef /Pad (a (b) \\) >>stream) /DecodeParms 9 0 R >> stream\nx\nendstream",
            b"<< /Type /XRef /Pad (>>) /DecodeParms 9 0 R >> stream\nx\nendstream",
            b"<< /Type /XRef % >>stream\n/DecodeParms 9 0 R >> stream\nx\nendstream",
            b"<< /Type /XRef /DecodeParms 9 0 R >> % a comment\nstream\nx\nendstream",
        ];
        let unread: Vec<_> = cases.iter().filter(|bad| !unsafe_file_streams(bad)).map(|bad| String::from_utf8_lossy(bad)).collect();
        assert!(unread.is_empty(), "{unread:#?}");
    }

    #[test]
    fn a_stream_in_text_the_scan_skips_is_still_read() {
        // The scan skips a stream's data, a string and a comment; the reader may still start at
        // a dictionary in them (a cross-reference offset can point at `1 0 obj` anywhere, its
        // repair scan tries a `<<` after white space), so they're read too.
        let bad = b"<< /Type /XRef /DecodeParms 9 0 R >> stream\nx\nendstream";
        // `data` as the data of a stream whose length holds all of it.
        let image = |data: &[u8]| [format!("1 0 obj << /Length {} >> stream\n", data.len()).as_bytes(), data, b"\nendstream endobj"].concat();
        let run = vec![b'<'; 1 << 16];
        let cases = [
            image(bad),
            [&b"(1 0 obj "[..], bad, b")"].concat(),
            [&b"(1 0 obj"[..], bad, b")"].concat(),
            [&b"% "[..], bad, b"\n"].concat(),
            [&b"<< /Note (1 0 obj "[..], bad, b") >>"].concat(),
            // #3924 review: right after a run of `<` in an image, and as the end of one (a
            // reader starting at the run's first `<<` reads it inside the others).
            image(&[&run[..], b"\n", bad].concat()),
            image(&[&run[..], b" /Type /XRef /DecodeParms 9 0 R >> stream\nx\nendstream"].concat()),
            // Strings in it that end far from their `(` (looked up, not read again): one, one
            // holding others, one from an escaped `(`, and the same after a flat area of
            // ` <<(` whose strings never end.
            image(&[&b"<< /Type /XRef /Pad ("[..], &[b'x'; 200], b") /DecodeParms 9 0 R >> stream\nx\nendstream"].concat()),
            image(&[&b"<< /Type /XRef /Pad (a (b) "[..], &[b'x'; 200], b" (c (d)) \\) ) /DecodeParms 9 0 R >> stream\nx\nendstream"].concat()),
            image(&[&b"<< /Type /XRef /Pad \\("[..], &[b'x'; 200], b") /DecodeParms 9 0 R >> stream\nx\nendstream"].concat()),
            image(&[&b" <<(".repeat(1 << 14)[..], b"\n", bad].concat()),
        ];
        let unread: Vec<_> =
            cases.iter().filter(|bad| !unsafe_file_streams(bad)).map(|bad| String::from_utf8_lossy(&bad[..bad.len().min(80)])).collect();
        assert!(unread.is_empty(), "{unread:#?}");
    }

    #[test]
    fn a_run_of_lt_in_an_image_a_string_or_a_comment_opens() {
        // #3924 review: every `<<` in a run of `<` (an image's flat area of grey 60) was read as a
        // dictionary to past the run, so 1 MiB of it spent the budget and the file was refused.
        // The reader starts a dictionary only at the run's first `<<`.
        let run = vec![b'<'; 1 << 20];
        let image = b"1 0 obj << /Type /XObject /Subtype /Image /Width 1024 /Height 1024 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1048576 >> stream\n";
        for (what, pdf) in [
            ("image", [&b"%PDF-1.7\n"[..], image, &run[..], b"\nendstream endobj\n"].concat()),
            ("string", [&b"%PDF-1.7\n2 0 obj ("[..], &run[..], b") endobj\n"].concat()),
            ("comment", [&b"%PDF-1.7\n% "[..], &run[..], b"\n"].concat()),
        ] {
            let t = std::time::Instant::now();
            assert!(!unsafe_file_streams(&pdf), "{what}");
            let took = t.elapsed();
            assert!(took < std::time::Duration::from_secs(5), "{what} took {took:?}");
        }
    }

    #[test]
    fn a_flat_area_that_opens_a_string_after_each_dictionary_opens() {
        // #3924 review: in a flat area of ` <<(` (CMYK or RGBA 32, 60, 60, 40) each `<<` was read
        // as a dictionary, and each read its never-ending string to 64 KiB past it: the budget
        // was spent and the file refused. A string ending far from its `(` is now read once.
        let noise: Vec<u8> = (0u32..1 << 18).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        for (what, data) in [
            (" <<(", b" <<(".repeat(1 << 18)),
            ("\\0<<(", b"\0<<(".repeat(1 << 18)),
            ("( <<", b"( <<".repeat(1 << 18)),
            (" <<( then noise", [&b" <<(".repeat(1 << 16)[..], &noise].concat()),
        ] {
            let pdf = [format!("1 0 obj << /Length {} >> stream\n", data.len()).as_bytes(), &data[..], b"\nendstream endobj\n"].concat();
            let t = std::time::Instant::now();
            assert!(!unsafe_file_streams(&pdf), "{what}");
            let took = t.elapsed();
            assert!(took < std::time::Duration::from_secs(5), "{what} took {took:?}");
        }
    }

    #[test]
    fn strings_and_comments_dense_with_dictionaries_are_scanned_in_bounded_time() {
        // Each `<<` in a string or comment the reader could start at is read as a dictionary of
        // its own, within a budget for the whole file.
        for unit in [&b"<<("[..], b"(<<", b"%<<\n", b"<</A (<</B 1>>stream) >>stream\n", b"<"] {
            let pdf = unit.repeat((1 << 20) / unit.len());
            let t = std::time::Instant::now();
            unsafe_file_streams(&pdf);
            let took = t.elapsed();
            assert!(took < std::time::Duration::from_secs(5), "{} took {took:?}", String::from_utf8_lossy(unit));
        }
        // An image's flat area of a colour whose bytes are `<<` and white space, `(` or `%` (a
        // reader may start at each `<<` after white space): read about once, and it opens.
        for pixel in [&b" <<"[..], b"<<(", b"<<%", b" <<%", b"\0<<\0"] {
            let data = pixel.repeat((1 << 20) / pixel.len());
            let pdf = [format!("1 0 obj << /Length {} >> stream\n", data.len()).as_bytes(), &data[..], b"\nendstream endobj\n"].concat();
            let t = std::time::Instant::now();
            assert!(!unsafe_file_streams(&pdf), "{}", String::from_utf8_lossy(pixel));
            let took = t.elapsed();
            assert!(took < std::time::Duration::from_secs(5), "{} took {took:?}", String::from_utf8_lossy(pixel));
        }
    }
}
