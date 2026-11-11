//! The chunk index: one flat table of every chunk, a list's children contiguous, built without
//! recursion. Payloads are not copied: an entry names a range of the input or of the bytes
//! inflated from `LIST cmpr` (layouts R1 to R4 in `PROVENANCE.md`).

use std::io::Read as _;

use crate::bytes::{Rd, u32_at};
use crate::limits::Budget;
use crate::{Error, Warning};

pub(crate) const LIST: [u8; 4] = *b"LIST";
const CMPR: [u8; 4] = *b"cmpr";
const CPNG: [u8; 4] = *b"CPng";
/// What follows `CPng` before the zlib stream (R3).
const CPNG_HEAD: [u8; 4] = [1, 0, 4, 0];

/// One chunk: 24 bytes. `start` and `len` give the payload (after a list's form type) in the
/// address space of the input followed by the inflated bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    pub(crate) id: [u8; 4],
    /// A list's form type; zeros for other chunks.
    pub(crate) form: [u8; 4],
    start: u32,
    len: u32,
    first: u32,
    count: u32,
}

impl Chunk {
    pub(crate) fn is_list(&self, form: &[u8; 4]) -> bool {
        self.id == LIST && &self.form == form
    }
}

/// Every chunk of a file, and the bytes they point into.
pub(crate) struct Index<'a> {
    input: &'a [u8],
    inflated: Vec<u8>,
    chunks: Vec<Chunk>,
    /// The version the form type names: hundreds, as `vrsn` stores it (0 when unknown).
    pub(crate) form_version: u16,
}

/// A list still to be split into its children.
struct Pending {
    entry: usize,
    /// Its children's bytes.
    start: usize,
    end: usize,
    /// The size pool of a compressed list's stream (`start..end` in the address space).
    pool: Option<(usize, usize)>,
    depth: usize,
}

/// The version the RIFF form type names (R1): `CDR5`..`CDR9`, `CDRA` (10) to `CDRH` (17) with `I`
/// unused, then `J` (18) on; `CDR ` is version 3. Some files use a lower-case `cdr`, which names
/// the same versions except that `cdr8` is the 8.01 variant.
pub(crate) fn form_version(form: [u8; 4]) -> Option<u16> {
    let [a, b, c, v] = form;
    match (&[a, b, c], v) {
        (b"cdr", b'8') => Some(801),
        (b"CDR" | b"cdr", b' ') => Some(300),
        (b"CDR" | b"cdr", b'1'..=b'9') => Some(100 * u16::from(v - b'0')),
        (b"CDR" | b"cdr", b'A'..=b'H') => Some(100 * u16::from(v - b'A' + 10)),
        (b"CDR" | b"cdr", b'J'..=b'W') => Some(100 * u16::from(v - b'J' + 18)),
        _ => None,
    }
}

/// Does `bytes` start like a RIFF `.cdr` drawing (not the `CDRX` exchange variant)?
pub fn is_cdr_riff(bytes: &[u8]) -> bool {
    bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12).and_then(|f| f.try_into().ok()).and_then(form_version).is_some()
}

impl<'a> Index<'a> {
    pub(crate) fn build(input: &'a [u8], budget: &mut Budget, warn: &mut Vec<Warning>) -> Result<Self, Error> {
        if input.len() > budget.limits.input {
            return Err(Error::Limit("input size"));
        }
        let mut r = Rd::new(input);
        if r.fourcc() != Some(*b"RIFF") {
            return Err(Error::NotCdr);
        }
        let declared = r.u32().ok_or(Error::NotCdr)? as usize;
        let form = r.fourcc().ok_or(Error::NotCdr)?;
        let version = form_version(form).ok_or(Error::NotCdr)?;
        // A file cut short keeps what it holds: its last chunk is truncated with a warning. A
        // length that ends inside the form type is damage too, and the chunks after it are read.
        let end = match declared.checked_add(8) {
            Some(e) if (12..=input.len()).contains(&e) => e,
            _ => {
                push(warn, Warning::Damaged);
                input.len()
            }
        };
        let mut ix = Index { input, inflated: Vec::new(), chunks: Vec::new(), form_version: version };
        ix.chunks.push(Chunk { id: *b"RIFF", form, start: 12, len: 0, first: 0, count: 0 });
        let mut queue = vec![Pending { entry: 0, start: 12, end, pool: None, depth: 0 }];
        while let Some(p) = queue.pop() {
            ix.split(p, &mut queue, budget, warn)?;
        }
        Ok(ix)
    }

    /// The bytes of the address range `start..end`, or `None` when it crosses the input's end.
    fn span(&self, start: usize, end: usize) -> Option<&[u8]> {
        let n = self.input.len();
        if end <= n {
            self.input.get(start..end)
        } else if start >= n {
            self.inflated.get(start - n..end - n)
        } else {
            None
        }
    }

    pub(crate) fn data(&self, c: &Chunk) -> &[u8] {
        let start = c.start as usize;
        self.span(start, start + c.len as usize).unwrap_or_default()
    }

    pub(crate) fn children(&self, c: &Chunk) -> &[Chunk] {
        let first = c.first as usize;
        self.chunks.get(first..first + c.count as usize).unwrap_or_default()
    }

    pub(crate) fn root(&self) -> Option<&Chunk> {
        self.chunks.first()
    }

    /// Split list `p` into its children, appended to the table together; their lists are queued.
    fn split(&mut self, p: Pending, queue: &mut Vec<Pending>, budget: &mut Budget, warn: &mut Vec<Warning>) -> Result<(), Error> {
        let first = self.chunks.len();
        let mut at = p.start;
        let mut lists = Vec::new();
        while at < p.end {
            let Some((chunk, next)) = self.read_chunk(at, p.end, p.pool) else {
                push(warn, Warning::Damaged);
                break;
            };
            budget.take_chunk()?;
            // A `LIST stlt` holds the style tables, not chunks (R4); the reader doesn't use them.
            if chunk.id == LIST && chunk.form != *b"stlt" {
                lists.push(self.chunks.len());
            }
            self.chunks.push(chunk);
            at = next;
        }
        let count = (self.chunks.len() - first) as u32;
        if let Some(parent) = self.chunks.get_mut(p.entry) {
            parent.first = first as u32;
            parent.count = count;
        }
        for i in lists {
            let Some(c) = self.chunks.get(i).copied() else { continue };
            if p.depth + 1 > budget.limits.depth {
                push(warn, Warning::TooDeep);
                continue;
            }
            let (start, end) = (c.start as usize, c.start as usize + c.len as usize);
            if c.form == CMPR && p.pool.is_none() {
                match self.inflate_cmpr(start, end, budget) {
                    Ok((s, e, pool)) => queue.push(Pending { entry: i, start: s, end: e, pool: Some(pool), depth: p.depth + 1 }),
                    Err(Error::Limit(what)) => return Err(Error::Limit(what)),
                    Err(_) => push(warn, Warning::Damaged),
                }
            } else {
                queue.push(Pending { entry: i, start, end, pool: p.pool, depth: p.depth + 1 });
            }
        }
        Ok(())
    }

    /// The chunk at `at` (before `end`) and where the next one starts. In a compressed stream the
    /// size field is an index into the size pool (R3).
    fn read_chunk(&self, at: usize, end: usize, pool: Option<(usize, usize)>) -> Option<(Chunk, usize)> {
        let head = self.span(at, at.checked_add(8)?.min(end))?;
        let id: [u8; 4] = head.get(..4)?.try_into().ok()?;
        let raw = u32_at(head, 4)? as usize;
        let size = match pool {
            None => raw,
            Some((ps, pe)) => {
                let off = raw.checked_mul(4)?.checked_add(ps)?;
                (off.checked_add(4)? <= pe).then_some(())?;
                u32_at(self.span(off, off + 4)?, 0)? as usize
            }
        };
        let body = at + 8;
        let body_end = body.checked_add(size).filter(|&e| e <= end)?;
        let next = body_end + (size & 1);
        let (form, start) = if id == LIST {
            let f: [u8; 4] = self.span(body, body.checked_add(4).filter(|&e| e <= body_end)?)?.try_into().ok()?;
            (f, body + 4)
        } else {
            ([0; 4], body)
        };
        let chunk = Chunk { id, form, start: u32::try_from(start).ok()?, len: u32::try_from(body_end - start).ok()?, first: 0, count: 0 };
        Some((chunk, next.min(end)))
    }

    /// Inflate the two blocks of a `LIST cmpr` body at `start..end` (R3) → the chunk stream and the
    /// size pool, as address ranges.
    fn inflate_cmpr(&mut self, start: usize, end: usize, budget: &mut Budget) -> Result<(usize, usize, (usize, usize)), Error> {
        let body = self.span(start, end).ok_or(Error::Damaged("compressed list"))?;
        let mut r = Rd::new(body);
        let c1 = r.u32().ok_or(Error::Damaged("compressed list"))? as usize;
        r.skip(4).ok_or(Error::Damaged("compressed list"))?;
        let c2 = r.u32().ok_or(Error::Damaged("compressed list"))? as usize;
        let block = |at: usize, len: usize| -> Result<&[u8], Error> {
            let b = body.get(at..at.checked_add(len).ok_or(Error::Damaged("compressed list"))?).ok_or(Error::Damaged("compressed list"))?;
            let mut h = Rd::new(b);
            if h.fourcc() != Some(CPNG) || h.fourcc() != Some(CPNG_HEAD) {
                return Err(Error::Damaged("compressed block"));
            }
            Ok(b.get(8..).unwrap_or_default())
        };
        let second_at = c1.checked_add(16).ok_or(Error::Damaged("compressed list"))?;
        let (first, second) = (block(16, c1)?.to_vec(), block(second_at, c2)?.to_vec());
        let base = self.input.len();
        let stream = self.inflate(&first, budget)?;
        let pool = self.inflate(&second, budget)?;
        Ok((base + stream.0, base + stream.1, (base + pool.0, base + pool.1)))
    }

    /// Inflate zlib `data` onto the end of the inflated bytes, within the budget → its range there.
    fn inflate(&mut self, data: &[u8], budget: &mut Budget) -> Result<(usize, usize), Error> {
        let at = self.inflated.len();
        let left = budget.inflate_left();
        let reader = flate2::read::ZlibDecoder::new(data);
        // One byte past what is left tells a stream that is too long from one that just fits.
        let got = match reader.take((left as u64).saturating_add(1)).read_to_end(&mut self.inflated) {
            Ok(n) => n,
            Err(_) => {
                self.inflated.truncate(at);
                return Err(Error::Damaged("compressed block"));
            }
        };
        if got > left {
            self.inflated.truncate(at);
            return Err(Error::Limit("inflated size"));
        }
        budget.take_inflated(got)?;
        if self.input.len().saturating_add(self.inflated.len()) > u32::MAX as usize {
            return Err(Error::Limit("inflated size"));
        }
        Ok((at, at + got))
    }
}

/// Add `w` to the warnings once.
pub(crate) fn push(warn: &mut Vec<Warning>, w: Warning) {
    if !warn.contains(&w) {
        warn.push(w);
    }
}
