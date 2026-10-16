//! AGD header, optional Macintosh chunk wrapper, zlib (or raw) record stream, name dictionary
//! and the record-type list. The drawing itself is parsed from the decompressed records.
//!
//! Versions 8 through 11 (MX) are read. Older FreeHand headers are recognised so they are not
//! handed to another importer, then refused.

use std::collections::HashMap;
use std::io::{ErrorKind, Read};

use flate2::read::ZlibDecoder;

use crate::Error;
use crate::cursor::{Cur, Phase};
use crate::records::{self, RawFile};

/// Caps on untrusted sizes. Past one of these the file is refused instead of allocating.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Compressed file bytes read from the AGD block.
    pub max_input: usize,
    /// Decompressed record stream.
    pub max_decompressed: usize,
    pub max_records: usize,
    pub max_dict: usize,
    /// Path nodes, list entries and property entries, each.
    pub max_items: usize,
    pub max_depth: usize,
    pub max_string: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input: 64 << 20,
            max_decompressed: 256 << 20,
            max_records: 250_000,
            max_dict: 8_192,
            max_items: 2_000_000,
            max_depth: 64,
            max_string: 1_048_576,
        }
    }
}

/// Product version this reader will parse, or a label for a FreeHand it will not.
enum Class {
    /// FreeHand 8, 9, 10 or 11 (MX). The number is the product version.
    Supported(u16),
    /// Recognisable FreeHand whose layout this reader does not implement. `{n}` in the error.
    Unsupported(&'static str),
}

/// `AGD` plus a version digit, `FH3`, or an early magic, at `i`.
fn magic_at(bytes: &[u8], i: usize) -> bool {
    match bytes.get(i..i + 4) {
        Some(b) => classify(b).is_some(),
        None => false,
    }
}

fn classify(magic: &[u8]) -> Option<Class> {
    match magic.get(..4)? {
        b"acf3" => Some(Class::Unsupported("1")),
        b"FHD2" => Some(Class::Unsupported("2")),
        [b'F', b'H', b'3', _] => Some(Class::Unsupported("3")),
        [b'A', b'G', b'D', d] => match d {
            // Public headers: AGD1 is FreeHand 4 and FreeHand 5. The digit formula would call it 6.
            b'0' => Some(Class::Unsupported("5")),
            b'1' => Some(Class::Unsupported("4 or 5")),
            b'2' => Some(Class::Unsupported("7")),
            b'3' => Some(Class::Supported(8)),
            b'4' => Some(Class::Supported(9)),
            b'5' => Some(Class::Supported(10)),
            b'6' => Some(Class::Supported(11)),
            _ => None,
        },
        _ => None,
    }
}

/// `FreeHand10` or `FreeHand11` inside the first 24 bytes, when present.
fn product_text(bytes: &[u8]) -> bool {
    let head = bytes.get(..24).unwrap_or(bytes);
    head.windows(10).any(|w| w == b"FreeHand11" || w == b"FreeHand10")
}

/// Byte offset of a FreeHand header, or `None` when `bytes` does not hold one.
pub(crate) fn locate(bytes: &[u8]) -> Option<usize> {
    if magic_at(bytes, 0) {
        return Some(0);
    }
    if bytes.first().copied() != Some(0x1C) {
        return None;
    }
    let mut pos = 0usize;
    let mut chunks = 0u32;
    while chunks < 4096 && pos + 5 <= bytes.len() && pos <= (8 << 20) {
        if bytes.get(pos).copied() != Some(0x1C) {
            return None;
        }
        let op = u16::from_be_bytes([bytes.get(pos + 1).copied()?, bytes.get(pos + 2).copied()?]);
        let flag = bytes.get(pos + 3).copied()?;
        let len_b = usize::from(bytes.get(pos + 4).copied()?);
        pos += 5;
        let payload = if flag == 0x80 {
            if len_b != 4 || pos + 4 > bytes.len() {
                return None;
            }
            let n = u32::from_be_bytes([
                bytes.get(pos).copied()?,
                bytes.get(pos + 1).copied()?,
                bytes.get(pos + 2).copied()?,
                bytes.get(pos + 3).copied()?,
            ]);
            pos += 4;
            usize::try_from(n).ok()?
        } else {
            len_b
        };
        if payload > bytes.len().saturating_sub(pos) {
            return None;
        }
        if op == 0x080A && magic_at(bytes, pos) {
            return Some(pos);
        }
        pos += payload;
        chunks += 1;
    }
    None
}

/// Whether `bytes` is recognisably FreeHand, including versions this reader refuses.
pub fn is_freehand(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ALDUS LZW") || product_text(bytes) || locate(bytes).is_some()
}

fn unsupported(n: &str) -> Error {
    // The same sentence is returned as `Err(format!(...))` below so the i18n scan sees it.
    format!("unsupported FreeHand version {n}: VectorCraft opens FreeHand 8 to MX (11); open it in a newer FreeHand and save, or export it as EPS, PDF or AI").into()
}

fn inflate(data: &[u8], max: usize) -> Result<Vec<u8>, Error> {
    let mut dec = ZlibDecoder::new(data);
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = match dec.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                return Err("the FreeHand file is cut off in its compressed data".into());
            }
            Err(_) => return Err("the FreeHand file's compressed data is damaged".into()),
        };
        if n == 0 {
            break;
        }
        if out.len().saturating_add(n) > max {
            return Err("the FreeHand file decompresses past the safety limit".into());
        }
        let chunk = buf.get(..n).ok_or_else(|| Error::new("the FreeHand file's compressed data is damaged"))?;
        out.extend_from_slice(chunk);
    }
    Ok(out)
}

fn read_dict(c: &mut Cur<'_>, version: u16, limits: &Limits) -> Result<HashMap<u16, String>, Error> {
    let count = usize::from(c.u16()?);
    let _gap = c.u16()?;
    if count > limits.max_dict {
        return Err("the FreeHand file claims a dictionary larger than can be read".into());
    }
    let mut names = HashMap::new();
    for _ in 0..count {
        let id = c.u16()?;
        if version <= 8 {
            c.skip(2)?;
        }
        let name = c.cstr(limits.max_string)?;
        if version <= 8 {
            // Two further C strings (often short). Their text is not a type name.
            let _ = c.cstr(limits.max_string)?;
            let _ = c.cstr(limits.max_string)?;
        }
        let text = String::from_utf8_lossy(name).into_owned();
        names.entry(id).or_insert(text);
    }
    Ok(names)
}

pub(crate) fn open(bytes: &[u8], limits: &Limits) -> Result<RawFile, Error> {
    if bytes.is_empty() {
        return Err("the FreeHand file is empty".into());
    }
    if bytes.len() > limits.max_input {
        return Err("the FreeHand file is larger than can be read".into());
    }
    if bytes.starts_with(b"ALDUS LZW") {
        return Err(unsupported("1"));
    }
    let Some(start) = locate(bytes) else {
        if product_text(bytes) || bytes.len() < 4 {
            return Err("the FreeHand file is cut off in the header".into());
        }
        return Err("not a FreeHand file".into());
    };
    let file = bytes.get(start..).ok_or_else(|| Error::new("not a FreeHand file"))?;
    let magic = file.get(..4).ok_or_else(|| Error::new("the FreeHand file is cut off in the header"))?;
    let version = match classify(magic) {
        Some(Class::Supported(v)) => v,
        Some(Class::Unsupported(n)) => {
            return Err(format!("unsupported FreeHand version {n}: VectorCraft opens FreeHand 8 to MX (11); open it in a newer FreeHand and save, or export it as EPS, PDF or AI").into());
        }
        None => return Err("not a FreeHand file".into()),
    };
    if file.len() < 12 {
        return Err("the FreeHand file is cut off in the header".into());
    }
    let mut c = Cur::phased(file, Phase::Header);
    c.skip(4)?;
    let _unidentified = c.u32()?;
    let data_len = usize::try_from(c.u32()?).map_err(|_| Error::new("the FreeHand file is larger than can be read"))?;
    if data_len < 12 {
        return Err("the FreeHand file is cut off in the header".into());
    }
    if data_len > limits.max_input {
        return Err("the FreeHand file is larger than can be read".into());
    }
    if file.len() < data_len {
        return Err("the FreeHand file's data block runs past the end of the file".into());
    }
    let payload = file.get(12..data_len).ok_or_else(|| Error::new("the FreeHand file is cut off in the header"))?;
    // Version 9 and later store zlib here. Older files store the records raw, so the
    // payload slice is the stream: nothing is copied before the records are parsed.
    let inflated = if version >= 9 { Some(inflate(payload, limits.max_decompressed)?) } else { None };
    let stream: &[u8] = match inflated.as_deref() {
        Some(buf) => buf,
        None => payload,
    };
    if stream.len() > limits.max_decompressed {
        return Err("the FreeHand file decompresses past the safety limit".into());
    }
    let tail_bytes = file.get(data_len..).ok_or_else(|| Error::new("the FreeHand file is cut off in the dictionary"))?;
    let mut tail = Cur::phased(tail_bytes, Phase::Dict);
    let dict = read_dict(&mut tail, version, limits)?;
    tail.set_phase(Phase::List);
    let rec_count = usize::try_from(tail.u32()?).map_err(|_| Error::new("the FreeHand file claims more records than can be read"))?;
    if rec_count > limits.max_records {
        return Err("the FreeHand file claims more records than can be read".into());
    }
    let mut types = Vec::new();
    types.try_reserve(rec_count).map_err(|_| Error::new("the FreeHand file claims more records than can be read"))?;
    for _ in 0..rec_count {
        let id = tail.u16()?;
        let name = dict.get(&id).cloned().unwrap_or_else(|| format!("type {id}"));
        types.push(name);
    }
    // Anything after the list (a `FlateDecode` trailer, when present) is not the drawing.
    let mut body = Cur::new(stream);
    let recs = records::parse(&mut body, &types, version, limits)?;
    let (block_id, props_id, fallback) = read_tail(body.rest());
    Ok(RawFile { version, recs, names: types, block_id, props_id, fallback })
}

/// The unlisted tail. The three ids and the fallback page maximum sit at fixed offsets
/// (a 16-bit id, not the escaped form: an escape would overlap the next field).
fn read_tail(data: &[u8]) -> (u32, u32, Option<(f64, f64)>) {
    let u16_at = |off: usize| -> u32 {
        match (data.get(off), data.get(off + 1)) {
            (Some(&a), Some(&b)) => u32::from(u16::from_be_bytes([a, b])),
            _ => 0,
        }
    };
    let fixed_at = |off: usize| -> Option<f64> {
        let b = data.get(off..off + 4)?;
        let n = i32::from_be_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]);
        Some(f64::from(n) / 65536.0)
    };
    let fallback = match (fixed_at(0x1A), fixed_at(0x1E)) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x > 0.0 && y > 0.0 && x < 1.0e7 && y < 1.0e7 => Some((x, y)),
        _ => None,
    };
    (u16_at(0), u16_at(2), fallback)
}
