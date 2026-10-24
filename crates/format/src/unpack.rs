//! Reading a file: its JSON, then the entries of the data after it (v4) one by one. A compressed
//! file is unpacked as a stream: the JSON into one buffer, each entry straight into its own, so
//! the file is never held unpacked as a whole. Limits apply per part ([`MAX_JSON`], [`MAX_BLOB`],
//! [`MAX_BLOBS`]), guarding against decompression bombs.

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::Arc;

use flate2::read::GzDecoder;

use crate::{BLOB_MAGIC, FormatError, MAX_BLOB, MAX_BLOBS, MAX_JSON, MAX_UNPACKED, is_compressed, split_tail};

/// Bytes unpacked at a time while looking for the end of the JSON.
const CHUNK: u64 = 64 << 10;
/// Most bytes reserved for an entry before any of it is unpacked.
const RESERVE_MAX: usize = 64 << 20;

fn damaged(e: impl std::fmt::Display) -> FormatError {
    FormatError::NotVectorcraft(format!("damaged compressed data: {e}"))
}

/// An entry's bytes (`None`: the data ends before it), shared by the entries naming them.
pub(crate) type Entry = Option<Arc<Vec<u8>>>;

/// A file opened for reading: its JSON, and where the data after it is.
pub(crate) struct Unpacked<'a> {
    json: Json<'a>,
    tail: Tail<'a>,
}

enum Json<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
    /// Unpacked whole (a compressed file whose JSON isn't followed by the marker where the writer
    /// puts it): `buf[..json]` is the JSON ([`Tail::Whole`] says where the data after it is).
    Whole {
        buf: Vec<u8>,
        json: usize,
    },
}

enum Tail<'a> {
    /// The data after the JSON, in memory (an uncompressed file, or one unpacked whole).
    Slice(&'a [u8]),
    /// In memory, in [`Json::Whole`]'s buffer from this offset.
    Whole(usize),
    /// Still to unpack from `file`: it starts at `at` in the unpacked stream; `read` holds what
    /// was unpacked past the marker already, and `rest` the stream from there.
    Stream {
        file: &'a [u8],
        at: u64,
        read: Vec<u8>,
        rest: Box<GzDecoder<&'a [u8]>>,
    },
    None,
}

impl<'a> Unpacked<'a> {
    /// Open `bytes` (compressed or not).
    pub(crate) fn new(bytes: &'a [u8]) -> Result<Self, FormatError> {
        if !is_compressed(bytes) {
            let (json, tail) = split_tail(bytes);
            return Ok(Self { json: Json::Borrowed(json), tail: Tail::Slice(tail) });
        }
        let mut dec = GzDecoder::new(bytes);
        let mut buf = Vec::new();
        let mut nul = None;
        loop {
            let start = buf.len();
            let n = (&mut dec).take(CHUNK).read_to_end(&mut buf).map_err(damaged)?;
            if nul.is_none() {
                nul = buf.get(start..).and_then(|b| b.iter().position(|&c| c == 0)).map(|p| p + start);
            }
            if n == 0 || nul.is_some_and(|z| buf.len() >= z + BLOB_MAGIC.len()) {
                break;
            }
            if buf.len() as u64 > MAX_JSON + CHUNK {
                return Err(FormatError::TooBig);
            }
        }
        let Some(z) = nul else {
            // No NUL, so no marker: all of it is JSON.
            if buf.len() as u64 > MAX_JSON {
                return Err(FormatError::TooBig);
            }
            return Ok(Self { json: Json::Owned(buf), tail: Tail::None });
        };
        // JSON holds no raw NUL, so the marker (which has one) starts right before the first NUL;
        // the writer puts it right after the JSON's last byte.
        if let Some(end) = z.checked_sub(1)
            && buf.get(end..).is_some_and(|r| r.starts_with(BLOB_MAGIC))
            && end.checked_sub(1).and_then(|k| buf.get(k)).is_some_and(|b| matches!(b, b'}' | b']' | b'"'))
        {
            if end as u64 > MAX_JSON {
                return Err(FormatError::TooBig);
            }
            let read = buf.split_off(end + BLOB_MAGIC.len());
            buf.truncate(end);
            let at = (end + BLOB_MAGIC.len()) as u64;
            return Ok(Self { json: Json::Owned(buf), tail: Tail::Stream { file: bytes, at, read, rest: Box::new(dec) } });
        }
        // Anything else (hand-made or damaged files) is unpacked whole and split as an
        // uncompressed file is.
        dec.take(MAX_UNPACKED + 1 - (buf.len() as u64).min(MAX_UNPACKED)).read_to_end(&mut buf).map_err(damaged)?;
        if buf.len() as u64 > MAX_UNPACKED {
            return Err(FormatError::TooBig);
        }
        let (json, tail) = split_tail(&buf);
        let (json, tail) = (json.len(), buf.len() - tail.len());
        Ok(Self { json: Json::Whole { buf, json }, tail: Tail::Whole(tail) })
    }

    /// The JSON text.
    pub(crate) fn json(&self) -> &[u8] {
        match &self.json {
            Json::Borrowed(j) => j,
            Json::Owned(j) => j,
            Json::Whole { buf, json, .. } => buf.get(..*json).unwrap_or_default(),
        }
    }

    /// The entries `[offset, length]` of the data after the JSON → each one's bytes, `None` for
    /// one that isn't there (the data ends before it). Entries naming the same bytes (only in
    /// hand-made files) share one buffer. Errors: the compressed data is damaged, or the entries
    /// of a compressed file are larger than [`MAX_BLOB`] each or [`MAX_BLOBS`] together, or those
    /// of an uncompressed file larger together than both [`MAX_BLOBS`] and the data, or the
    /// distinct entries found larger together than the data they are in (overlapping entries
    /// would copy the same bytes once each, #3574). Entries are read in the order of their
    /// offsets, each straight into its own buffer.
    pub(crate) fn entries(self, wanted: &[[u64; 2]]) -> Result<Vec<Entry>, FormatError> {
        self.entries_within(wanted, MAX_WORK)
    }

    /// [`Self::entries`], unpacking at most `work` bytes of a streamed file to find them.
    fn entries_within(self, wanted: &[[u64; 2]], work: u64) -> Result<Vec<Entry>, FormatError> {
        let mut out: Vec<Entry> = vec![None; wanted.len()];
        // The entries read so far, by `[offset, length]`, and the bytes copied for them.
        let mut done: HashMap<[u64; 2], Entry> = HashMap::new();
        let mut copied = 0u64;
        let mut in_memory = |tail: &[u8], out: &mut Vec<Entry>| {
            for (slot, &w) in out.iter_mut().zip(wanted) {
                if let Some(e) = done.get(&w) {
                    *slot = e.clone();
                    continue;
                }
                let bytes = range(w[0], w[1]).and_then(|(a, b)| tail.get(a..b));
                copied = copied.saturating_add(bytes.map_or(0, |b| b.len() as u64));
                if copied > tail.len() as u64 {
                    return Err(FormatError::TooBig);
                }
                *slot = bytes.map(|b| Arc::new(b.to_vec()));
                done.insert(w, slot.clone());
            }
            Ok(())
        };
        match self.tail {
            Tail::Slice(tail) => {
                within(wanted, u64::MAX, MAX_BLOBS.max(tail.len() as u64))?;
                in_memory(tail, &mut out)?;
            }
            Tail::Whole(at) => {
                within(wanted, MAX_BLOB, MAX_BLOBS)?;
                let buf = match &self.json {
                    Json::Whole { buf, .. } => buf.as_slice(),
                    _ => &[],
                };
                in_memory(buf.get(at..).unwrap_or_default(), &mut out)?;
            }
            Tail::None => in_memory(&[], &mut out)?,
            Tail::Stream { file, at, read, rest } => {
                within(wanted, MAX_BLOB, MAX_BLOBS)?;
                drop(self.json);
                let mut order: Vec<usize> = (0..wanted.len()).collect();
                order.sort_by_key(|&i| wanted.get(i).map_or(0, |w| w[0]));
                let mut s = Stream { r: Box::new(io::Cursor::new(read).chain(rest)), pos: 0, file, at, left: work };
                // How far into the data the stream has been: the bytes copied stay within it.
                let mut seen = 0u64;
                for i in order {
                    let Some(&w) = wanted.get(i) else { continue };
                    let Some(slot) = out.get_mut(i) else { continue };
                    if let Some(e) = done.get(&w) {
                        *slot = e.clone();
                        continue;
                    }
                    *slot = s.entry(w[0], w[1])?.map(Arc::new);
                    copied = copied.saturating_add(slot.as_ref().map_or(0, |e| e.len() as u64));
                    seen = seen.max(s.pos);
                    if copied > seen {
                        return Err(FormatError::TooBig);
                    }
                    done.insert(w, slot.clone());
                }
                // To the end, so the stream's checksum is verified as before.
                s.left = MAX_BLOBS;
                s.skip(MAX_BLOBS)?;
            }
        }
        Ok(out)
    }
}

/// Are the entries at most `each` bytes long each and `all` together? Checked before any is read.
fn within(wanted: &[[u64; 2]], each: u64, all: u64) -> Result<(), FormatError> {
    let mut total = 0u64;
    for &[_, len] in wanted {
        if len > each {
            return Err(FormatError::TooBig);
        }
        total = total.saturating_add(len);
    }
    if total > all { Err(FormatError::TooBig) } else { Ok(()) }
}

/// `offset..offset + len` as indices, when they fit.
fn range(offset: u64, len: u64) -> Option<(usize, usize)> {
    let a = usize::try_from(offset).ok()?;
    Some((a, a.checked_add(usize::try_from(len).ok()?)?))
}

/// The data after a compressed file's JSON, being unpacked.
struct Stream<'a> {
    r: Box<dyn Read + 'a>,
    /// Where `r` is in the data after the JSON.
    pos: u64,
    file: &'a [u8],
    /// Where the data after the JSON starts in the unpacked stream.
    at: u64,
    /// Bytes that may still be unpacked ([`MAX_WORK`] in all): the entries' own, the gaps before
    /// them, and the stream from its start again for each entry behind the one before.
    left: u64,
}

/// Most bytes unpacked to read a streamed file's entries: the entries and the gaps between them
/// ([`MAX_BLOBS`] in a file a writer made, where they follow each other), and the JSON once more.
/// The entries' lengths alone don't bound it: one byte asked for at a far offset unpacks all
/// that lies before it, and each entry behind the last one starts the stream over (#1699).
const MAX_WORK: u64 = MAX_BLOBS + MAX_JSON;

impl Stream<'_> {
    /// Count `n` bytes unpacked against the budget.
    fn spend(&mut self, n: u64) -> Result<(), FormatError> {
        self.left = self.left.checked_sub(n).ok_or(FormatError::TooBig)?;
        Ok(())
    }

    /// Unpack and drop up to `n` bytes → how many there were.
    fn skip(&mut self, n: u64) -> Result<u64, FormatError> {
        // One byte past the budget at most, so going over it is seen without doing the work.
        let skipped = io::copy(&mut (&mut self.r).take(n.min(self.left.saturating_add(1))), &mut io::sink()).map_err(damaged)?;
        self.spend(skipped)?;
        self.pos += skipped;
        Ok(skipped)
    }

    /// The entry at `offset`, `len` bytes long (`None`: the data ends before it does).
    fn entry(&mut self, offset: u64, len: u64) -> Result<Option<Vec<u8>>, FormatError> {
        if offset < self.pos {
            // Entries out of order or overlapping (only hand-made files): unpack again from the
            // start.
            let mut dec = GzDecoder::new(self.file);
            let skipped = io::copy(&mut (&mut dec).take(self.at.min(self.left.saturating_add(1))), &mut io::sink()).map_err(damaged)?;
            self.spend(skipped)?;
            if skipped < self.at {
                return Ok(None);
            }
            self.r = Box::new(dec);
            self.pos = 0;
        }
        let gap = offset - self.pos;
        if self.skip(gap)? < gap {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        let n = usize::try_from(len).map_err(|_| FormatError::TooBig)?;
        // The length is the file's word: reserve up to a bound, and let real data grow the rest.
        bytes.try_reserve_exact(n.min(RESERVE_MAX)).map_err(|_| FormatError::TooBig)?;
        let got = (&mut self.r).take(len.min(self.left.saturating_add(1))).read_to_end(&mut bytes).map_err(damaged)?;
        self.spend(got as u64)?;
        self.pos += got as u64;
        Ok((got as u64 == len).then_some(bytes))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(bytes).unwrap();
        gz.finish().unwrap()
    }

    /// #1699: reading a streamed file's entries unpacks a bounded number of bytes, whatever
    /// their offsets: a far offset and entries behind one another are refused past the budget.
    #[test]
    fn a_streamed_files_entries_are_found_within_a_work_budget() {
        let tail: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let mut file = b"{}".to_vec();
        file.extend(crate::BLOB_MAGIC);
        file.extend(&tail);
        let file = gzip(&file);
        let read = |wanted: &[[u64; 2]], work: u64| {
            let u = Unpacked::new(&file).unwrap();
            assert!(matches!(u.tail, Tail::Stream { .. }), "read as a stream");
            u.entries_within(wanted, work)
        };
        // Entries one after the other: the tail is unpacked once.
        let parts = read(&[[0, 1000], [1000, 3096]], 4096).unwrap();
        assert_eq!(parts, vec![Some(Arc::new(tail[..1000].to_vec())), Some(Arc::new(tail[1000..].to_vec()))]);
        // One byte at the far end is the whole tail's work.
        assert_eq!(read(&[[4095, 1]], 4096).unwrap(), vec![Some(Arc::new(vec![tail[4095]]))]);
        assert!(matches!(read(&[[4095, 1]], 4000), Err(FormatError::TooBig)));
        // Entries over the same bytes start the stream again each time: counted, then refused.
        let same: Vec<[u64; 2]> = (0..50).map(|i| [4000, 1 + i]).collect();
        assert!(read(&same, 50 * (4096 + 64)).is_ok());
        assert!(matches!(read(&same, 3 * 4096), Err(FormatError::TooBig)));
        // An offset past the data is still just missing, and the default budget reads all of it.
        assert_eq!(read(&[[1 << 40, 1]], 1 << 20).unwrap(), vec![None]);
        assert_eq!(Unpacked::new(&file).unwrap().entries(&[[0, 4096]]).unwrap(), vec![Some(Arc::new(tail.clone()))]);
    }

    #[test]
    fn entries_sharing_bytes_obey_the_limits_in_memory_too() {
        // Each entry within MAX_BLOB, together past MAX_BLOBS: many references to the same bytes.
        let many = vec![[0, MAX_BLOB]; 5];
        // Uncompressed.
        assert!(matches!(Unpacked::new(b"{}").unwrap().entries(&many), Err(FormatError::TooBig)));
        // Compressed, unpacked whole (no marker right after the JSON).
        let whole = gzip(b"{}\0x");
        assert!(matches!(Unpacked::new(&whole).unwrap().entries(&many), Err(FormatError::TooBig)));
        assert!(matches!(Unpacked::new(&whole).unwrap().entries(&[[0, MAX_BLOB + 1]]), Err(FormatError::TooBig)));
        // Uncompressed files keep no per-entry limit: a length the data doesn't have is just missing.
        assert_eq!(Unpacked::new(b"{}").unwrap().entries(&[[0, MAX_BLOB + 1]]).unwrap(), vec![None]);
        // Within the limits, entries naming the same bytes share them.
        let file = b"{}\n\0VCBLOBS\0abcd";
        let shared = Unpacked::new(file).unwrap().entries(&[[0, 4], [0, 4]]).unwrap();
        assert_eq!(shared, vec![Some(Arc::new(b"abcd".to_vec())), Some(Arc::new(b"abcd".to_vec()))]);
        assert!(Arc::ptr_eq(shared[0].as_ref().unwrap(), shared[1].as_ref().unwrap()));
    }

    /// The bytes held for `entries`, each buffer counted once however many entries share it.
    macro_rules! stored {
        ($entries:expr) => {{
            let mut bufs = std::collections::HashSet::new();
            $entries.iter().flatten().filter(|e| bufs.insert(e.as_ptr())).map(|e| e.len()).sum::<usize>()
        }};
    }

    /// #3574: entries naming the same bytes, or overlapping ones, would copy them once each, far
    /// past the data the file has: they share one buffer or are refused, in memory and streamed.
    #[test]
    fn entries_sharing_bytes_hold_no_more_than_the_data() {
        let tail: Vec<u8> = (0..65536u32).map(|i| (i % 251) as u8).collect();
        let n = tail.len() as u64;
        let same = vec![[0, n]; 50];
        let overlapping: Vec<[u64; 2]> = (0..50).map(|i| [i, n - i]).collect();
        let with = |json: &[u8]| {
            let mut file = json.to_vec();
            file.extend(crate::BLOB_MAGIC);
            file.extend(&tail);
            file
        };
        let plain = with(b"{}");
        // Compressed, unpacked whole (the marker isn't right after a `}`), and streamed.
        let whole = gzip(&with(b"1"));
        let streamed = gzip(&plain);
        for file in [&plain, &whole, &streamed] {
            let u = Unpacked::new(file).unwrap();
            let kind = match u.tail {
                Tail::Slice(_) => "in place",
                Tail::Whole(_) => "whole",
                Tail::Stream { .. } => "streamed",
                Tail::None => "no data",
            };
            match u.entries(&same) {
                Ok(e) => {
                    assert!(stored!(e) <= tail.len(), "{kind}: {} bytes held for {} of data", stored!(e), tail.len());
                    assert!(e.iter().all(|e| e.as_deref().map(|b| &b[..]) == Some(&tail[..])), "{kind}");
                }
                Err(e) => assert!(matches!(e, FormatError::TooBig), "{kind}: {e}"),
            }
            match Unpacked::new(file).unwrap().entries(&overlapping) {
                Ok(e) => assert!(stored!(e) <= tail.len(), "{kind}: {} bytes held for {} of data", stored!(e), tail.len()),
                Err(e) => assert!(matches!(e, FormatError::TooBig), "{kind}: {e}"),
            }
            // One entry over all of it is fine.
            let one = Unpacked::new(file).unwrap().entries(&same[..1]).unwrap();
            assert_eq!(one[0].as_deref().map(|b| &b[..]), Some(&tail[..]), "{kind}");
        }
    }
}
