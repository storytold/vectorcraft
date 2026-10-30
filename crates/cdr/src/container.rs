//! CDR RIFF, compressed RIFF and ZIP stream containers.

use crate::reader::Reader;
use flate2::read::{DeflateDecoder, ZlibDecoder};
use std::{collections::BTreeMap, io::Read};

const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_RECORDS: usize = 200_000;
const MAX_DEPTH: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    pub id: [u8; 4],
    pub list: Option<[u8; 4]>,
    pub data: Vec<u8>,
    pub children: Vec<Chunk>,
}
impl Chunk {
    /// Find the first direct child with the requested FourCC.
    pub fn child(&self, id: &[u8; 4]) -> Option<&Chunk> {
        self.children.iter().find(|c| &c.id == id)
    }
}

/// Convert an exactly four-byte field into a FourCC, rejecting truncated tags.
fn four(bytes: &[u8]) -> Result<[u8; 4], String> {
    bytes.try_into().map_err(|_| "truncated CDR FourCC".into())
}

/// Recognize a complete native RIFF header, excluding the `CDRX` form.
fn signature(bytes: &[u8]) -> bool {
    bytes.get(..4) == Some(b"RIFF") && bytes.get(8..11).is_some_and(|s| s.eq_ignore_ascii_case(b"CDR")) && bytes.get(11).is_some_and(|c| *c != b'X')
}

/// Sniff a bounded native header, reading only a 12-byte prefix from ZIP roots.
pub(crate) fn recognize(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_BYTES {
        return false;
    }
    signature(bytes)
        || zip_directory(bytes).is_ok_and(|d| {
            let Some(e) = d.get("content/riffData.cdr").or_else(|| d.get("content/root.dat")) else { return false };
            zip_prefix(bytes, e).is_ok_and(|root| signature(&root))
        })
}

struct ZipEntry {
    offset: usize,
    compressed: usize,
    size: usize,
    method: u16,
    crc: u32,
    flags: u16,
    name: String,
    data_end: usize,
}

/// Index a bounded central directory, rejecting duplicate names and unsupported archive layouts.
fn zip_directory(bytes: &[u8]) -> Result<BTreeMap<String, ZipEntry>, String> {
    if !bytes.starts_with(b"PK") {
        return Err("not a ZIP container".into());
    }
    let start = bytes.len().saturating_sub(65557);
    let end = bytes.len().saturating_sub(21);
    let pos = (start..end).rev().find(|&p| bytes.get(p..p + 4) == Some(b"PK\x05\x06")).ok_or("CDR ZIP directory missing")?;
    let mut r = Reader::new(bytes).at(pos + 4)?;
    if r.u16()? != 0 || r.u16()? != 0 {
        return Err("split CDR ZIP archives are unsupported".into());
    }
    let disk_count = r.u16()?;
    let count = r.u16()?;
    let size = r.u32()? as usize;
    let offset = r.u32()? as usize;
    let comment = r.u16()? as usize;
    if count == u16::MAX || size == u32::MAX as usize || offset == u32::MAX as usize {
        return Err("ZIP64 CDR archives are unsupported".into());
    }
    if count > 4096 || count != disk_count {
        return Err("CDR ZIP entry limit exceeded".into());
    }
    if pos.checked_add(22 + comment) != Some(bytes.len()) || offset.checked_add(size).is_none_or(|v| v > pos) {
        return Err("invalid CDR ZIP directory bounds".into());
    }
    let directory = bytes.get(offset..offset + size).ok_or("invalid CDR ZIP directory")?;
    let mut r = Reader::new(directory);
    let mut entries = BTreeMap::new();
    for _ in 0..count {
        if r.take(4)? != b"PK\x01\x02" {
            return Err("invalid CDR ZIP directory entry".into());
        }
        r.skip(4)?;
        let flags = r.u16()?;
        if flags & 1 != 0 {
            return Err("encrypted CDR ZIP entries are unsupported".into());
        }
        let method = r.u16()?;
        r.skip(4)?;
        let crc = r.u32()?;
        let compressed = r.u32()? as usize;
        let size = r.u32()? as usize;
        let name_len = r.u16()? as usize;
        let extra_len = r.u16()? as usize;
        let comment_len = r.u16()? as usize;
        if r.u16()? != 0 {
            return Err("split CDR ZIP entry".into());
        }
        r.skip(6)?;
        let offset = r.u32()? as usize;
        let name = std::str::from_utf8(r.take(name_len)?).map_err(|_| "non-UTF-8 CDR ZIP entry name")?.to_owned();
        r.skip(extra_len + comment_len)?;
        if entries.insert(name.clone(), ZipEntry { offset, compressed, size, method, crc, flags, name, data_end: 0 }).is_some() {
            return Err("duplicate CDR ZIP entry".into());
        }
    }
    for entry in entries.values_mut() {
        entry.data_end = offset;
    }
    Ok(entries)
}

/// Expand a stream against the shared output budget, returning an error on excess.
fn inflate(reader: impl Read, remaining: &mut usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    reader.take(*remaining as u64 + 1).read_to_end(&mut out).map_err(|e| format!("CDR decompression failed: {e}"))?;
    *remaining = remaining.checked_sub(out.len()).ok_or("CDR decompressed data limit exceeded")?;
    Ok(out)
}

/// Validate local/header agreement and payload bounds before borrowing compressed entry bytes.
fn zip_body<'a>(bytes: &'a [u8], e: &ZipEntry) -> Result<&'a [u8], String> {
    let mut r = Reader::new(bytes).at(e.offset)?;
    if r.take(4)? != b"PK\x03\x04" {
        return Err("invalid CDR ZIP local header".into());
    }
    r.skip(2)?;
    let flags = r.u16()?;
    let method = r.u16()?;
    r.skip(4)?;
    let crc = r.u32()?;
    let compressed = r.u32()? as usize;
    let size = r.u32()? as usize;
    let names = r.u16()? as usize;
    let extra = r.u16()? as usize;
    let name = r.take(names)?;
    r.skip(extra)?;
    if flags != e.flags
        || method != e.method
        || name != e.name.as_bytes()
        || (flags & 8 == 0 && (crc != e.crc || compressed != e.compressed || size != e.size))
    {
        return Err("CDR ZIP local and directory headers disagree".into());
    }
    if r.pos.checked_add(e.compressed).is_none_or(|end| end > e.data_end) {
        return Err("CDR ZIP entry overlaps its directory".into());
    }
    r.take(e.compressed)
}

/// Recognition needs only the RIFF header; full expansion and CRC checks belong to loading.
fn zip_prefix(bytes: &[u8], e: &ZipEntry) -> Result<[u8; 12], String> {
    if !(12..=MAX_BYTES).contains(&e.size) {
        return Err("invalid CDR ZIP root size".into());
    }
    let body = zip_body(bytes, e)?;
    let mut prefix = [0; 12];
    match e.method {
        0 => prefix.copy_from_slice(body.get(..12).ok_or("truncated CDR ZIP root")?),
        8 => DeflateDecoder::new(body).read_exact(&mut prefix).map_err(|e| format!("CDR prefix decompression failed: {e}"))?,
        _ => return Err("unsupported CDR ZIP compression method".into()),
    }
    Ok(prefix)
}

/// Fully decode an entry, charging expansion and verifying its declared size and CRC.
fn zip_read(bytes: &[u8], e: &ZipEntry, remaining: &mut usize) -> Result<Vec<u8>, String> {
    if e.size > *remaining {
        return Err("CDR decompressed data limit exceeded".into());
    }
    let body = zip_body(bytes, e)?;
    let out = match e.method {
        0 => {
            *remaining = remaining.checked_sub(body.len()).ok_or("CDR expansion limit exceeded")?;
            body.to_vec()
        }
        8 => inflate(DeflateDecoder::new(body), remaining)?,
        _ => return Err("unsupported CDR ZIP compression method".into()),
    };
    if out.len() != e.size || crc32fast::hash(&out) != e.crc {
        return Err("CDR ZIP size or CRC mismatch".into());
    }
    Ok(out)
}

/// Load native RIFF or ZIP/external streams into checked chunks and resolve the format version.
pub(crate) fn load(bytes: &[u8]) -> Result<(Vec<Chunk>, u32), String> {
    if bytes.len() > MAX_BYTES {
        return Err("CDR file exceeds 128 MiB input limit".into());
    }
    if bytes.starts_with(b"WL") {
        return Err("CorelDRAW 1/2 Waldo files are unsupported; save as CDR 6 or newer".into());
    }
    let mut remaining = MAX_BYTES;
    let mut streams = Vec::new();
    let root;
    let data = if signature(bytes) {
        bytes
    } else {
        let directory = zip_directory(bytes)?;
        let e = directory.get("content/riffData.cdr").or_else(|| directory.get("content/root.dat")).ok_or("ZIP has no native CorelDRAW drawing")?;
        root = zip_read(bytes, e, &mut remaining)?;
        if let Some(list) = directory.get("content/dataFileList.dat") {
            let names = zip_read(bytes, list, &mut remaining)?;
            let names = std::str::from_utf8(&names).map_err(|_| "invalid CDR data stream list")?;
            for name in names.lines().filter(|s| !s.is_empty()) {
                if streams.len() >= 4096 {
                    return Err("CDR stream limit exceeded".into());
                }
                let key = format!("content/data/{name}");
                let e = directory.get(&key).ok_or("missing external CDR data stream")?;
                streams.push(zip_read(bytes, e, &mut remaining)?);
            }
        }
        &root
    };
    if !signature(data) {
        return Err("invalid CorelDRAW RIFF signature".into());
    }
    let mut r = Reader::new(data);
    r.skip(4)?;
    let len = r.u32()? as usize;
    if len < 4 {
        return Err("invalid CDR RIFF length".into());
    }
    let form = r.take(4)?;
    let c = form.get(3).copied().ok_or("missing CDR version")?;
    let mut version = match c {
        b'1'..=b'9' => (c - b'0') as u32 * 100,
        b'A'..=b'H' => (c - b'A' + 10) as u32 * 100,
        b'J'..=b'Z' => (c - b'J' + 18) as u32 * 100,
        _ => 0,
    };
    // vrsn is authoritative and corrects the skipped 'I' signature convention.
    let body = data.get(12..8usize.checked_add(len).ok_or("CDR size overflow")?).ok_or("truncated CDR RIFF")?;
    let mut p = 0usize;
    while let Some(head) = body.get(p..p.saturating_add(8)) {
        let mut h = Reader::new(head);
        let id = h.take(4)?;
        let n = h.u32()? as usize;
        if id == b"vrsn" {
            if let Some(v) = body.get(p + 8..p + 10) {
                version = Reader::new(v).u16()? as u32;
            }
            break;
        }
        if id == b"fver" {
            let end = p.checked_add(8).and_then(|v| v.checked_add(n)).ok_or("CDR version size overflow")?;
            let payload = body.get(p + 8..end).ok_or("truncated CDR version record")?;
            let mut v = Reader::new(payload);
            if n == 16 && v.u32()? == u32::MAX {
                let size = v.u32()? as usize;
                version = Reader::new(v.take(size)?).u16()? as u32;
            } else {
                version = Reader::new(payload).u16()? as u32;
            }
            break;
        }
        let Some(next) = p.checked_add(8).and_then(|a| a.checked_add(n)).and_then(|a| a.checked_add(n % 2)) else { break };
        if next <= p {
            break;
        }
        p = next;
    }
    if !(600..=2700).contains(&version) {
        return Err(format!("unsupported CDR format version {version}; supported record layouts are 6 through 27"));
    }
    let mut parser = Parser { version, streams: &streams, remaining: &mut remaining, copied: MAX_BYTES, visited: MAX_BYTES * 4, count: 0 };
    Ok((parser.chunks(body, None, 0)?, version))
}

struct Parser<'a> {
    version: u32,
    streams: &'a [Vec<u8>],
    remaining: &'a mut usize,
    copied: usize,
    visited: usize,
    count: usize,
}
impl Parser<'_> {
    /// Parse RIFF lists and external references under nesting, traversal, expansion and copy quotas.
    fn chunks(&mut self, data: &[u8], sizes: Option<&[u32]>, depth: usize) -> Result<Vec<Chunk>, String> {
        if depth > MAX_DEPTH {
            return Err("CDR nesting limit exceeded".into());
        }
        self.visited = self.visited.checked_sub(data.len()).ok_or("CDR cumulative traversal limit exceeded")?;
        let mut r = Reader::new(data);
        let mut out = Vec::new();
        while r.pos < data.len() {
            // Inspect a zero tail once. Repeated zero FourCC records are invalid,
            // otherwise checking the whole remaining stream per record is quadratic.
            let tail = data.get(r.pos..).ok_or("CDR cursor outside list")?;
            if tail.len() < 8 || tail.get(..4) == Some(&[0, 0, 0, 0]) {
                if tail.iter().all(|b| *b == 0) {
                    break;
                }
                return Err("invalid CDR zero tag or trailing data".into());
            }
            self.count += 1;
            if self.count > MAX_RECORDS {
                return Err("CDR record limit exceeded".into());
            }
            let id = four(r.take(4)?)?;
            let n = r.u32()?;
            let n = if let Some(sizes) = sizes { *sizes.get(n as usize).ok_or("invalid compressed CDR length index")? as usize } else { n as usize };
            let raw = r.take(n)?;
            if n % 2 != 0 {
                r.skip(1)?;
            }
            let mut body = raw;
            if self.version >= 1600 && n == 16 {
                let mut ext = Reader::new(raw);
                let stream = ext.u32()?;
                let size = ext.u32()? as usize;
                if stream == u32::MAX {
                    body = ext.take(size)?;
                } else {
                    let offset = ext.u32()? as usize;
                    let end = offset.checked_add(size).ok_or("CDR external size overflow")?;
                    body = self.streams.get(stream as usize).and_then(|s| s.get(offset..end)).ok_or("invalid external CDR stream reference")?;
                }
            }
            let mut chunk = Chunk { id, list: None, data: Vec::new(), children: Vec::new() };
            if &id == b"LIST" || &id == b"RIFF" {
                let mut list = Reader::new(body);
                let kind = four(list.take(4)?)?;
                chunk.list = Some(kind);
                if &kind == b"cmpr" {
                    let c1 = list.u32()? as usize;
                    let u1 = list.u32()? as usize;
                    let c2 = list.u32()? as usize;
                    let u2 = list.u32()? as usize;
                    if u1 > *self.remaining || u2 > self.remaining.saturating_sub(u1) {
                        return Err("CDR expansion limit exceeded".into());
                    }
                    let first = list.take(c1)?;
                    let second = list.take(c2)?;
                    let expand = |s: &[u8], rem: &mut usize| -> Result<Vec<u8>, String> {
                        let mut r = Reader::new(s);
                        if r.take(8)? != b"CPng\x01\x00\x04\x00" {
                            return Err("invalid compressed CDR header".into());
                        }
                        inflate(ZlibDecoder::new(r.take(s.len() - 8)?), rem)
                    };
                    let data = expand(first, self.remaining)?;
                    let lengths = expand(second, self.remaining)?;
                    if data.len() != u1 || lengths.len() != u2 || lengths.len() % 4 != 0 {
                        return Err("compressed CDR size mismatch".into());
                    }
                    let mut lr = Reader::new(&lengths);
                    let mut pool = Vec::new();
                    while lr.pos < lengths.len() {
                        pool.push(lr.u32()?);
                    }
                    chunk.children = self.chunks(&data, Some(&pool), depth + 1)?;
                } else if &kind == b"stlt" {
                    let data = list.take(body.len() - 4)?;
                    self.copied = self.copied.checked_sub(data.len()).ok_or("CDR materialized data limit exceeded")?;
                    chunk.data = data.to_vec();
                } else {
                    chunk.children = self.chunks(list.take(body.len() - 4)?, sizes, depth + 1)?;
                }
            } else {
                self.copied = self.copied.checked_sub(body.len()).ok_or("CDR materialized data limit exceeded")?;
                chunk.data = body.to_vec();
            }
            out.push(chunk);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::io::Write;
    /// Encode an original RIFF chunk with its required even-byte padding.
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = id.to_vec();
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(body);
        if !body.len().is_multiple_of(2) {
            b.push(0);
        }
        b
    }
    /// Wrap test records in a native CDR 12 RIFF header.
    fn riff(body: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        b.extend_from_slice(b"CDRC");
        b.extend_from_slice(body);
        b
    }
    /// Create the public `CPng`/zlib representation used by compressed RIFF streams.
    fn compressed(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        let mut b = b"CPng\x01\0\x04\0".to_vec();
        b.extend(enc.finish().unwrap());
        b
    }
    /// Replace generated chunk lengths with indices into a shared compressed-length pool.
    fn indexed(bytes: &[u8], sizes: &mut Vec<u32>) -> Vec<u8> {
        let mut r = Reader::new(bytes);
        let mut result = Vec::new();
        while r.pos < bytes.len() {
            let id = r.take(4).unwrap();
            let n = r.u32().unwrap() as usize;
            let body = r.take(n).unwrap();
            r.skip(n % 2).unwrap();
            let body = if id == b"LIST" {
                let mut b = body[..4].to_vec();
                b.extend(indexed(&body[4..], sizes));
                b
            } else {
                body.to_vec()
            };
            let ix = sizes.len() as u32;
            sizes.push(body.len() as u32);
            result.extend_from_slice(id);
            result.extend_from_slice(&ix.to_le_bytes());
            result.extend(&body);
            if body.len() % 2 != 0 {
                result.push(0);
            }
        }
        result
    }
    /// Build a single-entry stored ZIP from original test bytes.
    pub(super) fn zip(name: &str, data: &[u8]) -> Vec<u8> {
        zip_with_method(name, data, 0)
    }
    /// Build matching local/central ZIP headers for stored or raw-deflated test bytes.
    fn zip_with_method(name: &str, data: &[u8], method: u16) -> Vec<u8> {
        let body = if method == 8 {
            let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(data).unwrap();
            enc.finish().unwrap()
        } else {
            data.to_vec()
        };
        let crc = crc32fast::hash(data);
        let size = data.len() as u32;
        let compressed = body.len() as u32;
        let n = name.len() as u16;
        let mut b = b"PK\x03\x04".to_vec();
        b.extend_from_slice(&[20, 0, 0, 0]);
        b.extend_from_slice(&method.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        for v in [crc, compressed, size] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&n.to_le_bytes());
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(name.as_bytes());
        b.extend_from_slice(&body);
        let offset = b.len() as u32;
        let mut c = b"PK\x01\x02".to_vec();
        c.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        c.extend_from_slice(&method.to_le_bytes());
        c.extend_from_slice(&[0; 4]);
        for v in [crc, compressed, size] {
            c.extend_from_slice(&v.to_le_bytes());
        }
        c.extend_from_slice(&n.to_le_bytes());
        c.extend_from_slice(&[0; 16]);
        c.extend_from_slice(name.as_bytes());
        let len = c.len() as u32;
        b.extend(c);
        b.extend_from_slice(b"PK\x05\x06");
        b.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        b.extend_from_slice(&len.to_le_bytes());
        b.extend_from_slice(&offset.to_le_bytes());
        b.extend_from_slice(&[0, 0]);
        b
    }
    #[test]
    /// Require compressed length-pool decoding to preserve the generated native document.
    fn compressed_length_pool_recovers_same_native_drawing() {
        let plain = crate::synth::rectangle();
        let mut sizes = Vec::new();
        let data = indexed(&plain[12..], &mut sizes);
        let lengths: Vec<u8> = sizes.into_iter().flat_map(u32::to_le_bytes).collect();
        let first = compressed(&data);
        let second = compressed(&lengths);
        let mut body = b"cmpr".to_vec();
        for n in [first.len(), data.len(), second.len(), lengths.len()] {
            body.extend_from_slice(&(n as u32).to_le_bytes());
        }
        body.extend(first);
        body.extend(second);
        let zipped = riff(&chunk(b"LIST", &body));
        let p = crate::import(&plain).unwrap();
        let z = crate::import(&zipped).unwrap();
        assert_eq!(p.document, z.document);
    }
    #[test]
    /// Reject unrelated roots and conflicting local ZIP flags or compression methods.
    fn zip_recognition_checks_native_root_and_rejects_local_header_mismatch() {
        let b = zip("content/root.dat", &crate::synth::rectangle());
        assert!(recognize(&b));
        assert!(crate::import(&b).is_ok());
        assert!(!recognize(&zip("content/root.dat", b"plain unrelated file")));
        let mut mismatch = b.clone();
        mismatch[6] = 1;
        assert!(!recognize(&mismatch));
        assert!(crate::import(&mismatch).is_err());
        let mut mismatch = b;
        mismatch[8] = 99;
        assert!(!recognize(&mismatch));
    }
    #[test]
    /// Keep weak legacy prefixes out of sniffing while retaining the Waldo import diagnostic.
    fn recognition_requires_a_full_riff_signature_and_does_not_guess_waldo() {
        assert!(!recognize(b"WL"));
        assert!(!recognize(b"WL unrelated file"));
        assert!(!recognize(b"RIFF\x04\0\0\0CDR"));
        assert!(load(b"WL").unwrap_err().contains("Waldo"));
    }
    #[test]
    /// Exercise stored/deflated sniffing and defer full CRC validation to loading.
    fn zip_recognition_reads_only_the_signature_and_loading_checks_crc() {
        let root = crate::synth::rectangle();
        for method in [0, 8] {
            let mut bytes = zip_with_method("content/riffData.cdr", &root, method);
            assert!(recognize(&bytes));
            assert!(crate::import(&bytes).is_ok());
            // Matching headers with a wrong CRC must still sniff as CDR, but never load.
            let directory = bytes.windows(4).position(|s| s == b"PK\x01\x02").unwrap();
            let wrong_crc = (crc32fast::hash(&root) ^ 1).to_le_bytes();
            bytes[14..18].copy_from_slice(&wrong_crc);
            bytes[directory + 16..directory + 20].copy_from_slice(&wrong_crc);
            assert!(recognize(&bytes));
            assert!(load(&bytes).unwrap_err().contains("CRC"));
            assert!(!recognize(&zip_with_method("content/root.dat", b"RIFF", method)));
        }
    }
    #[test]
    /// Stop decompression when its output would exceed the supplied budget.
    fn expansion_budget_stops_before_large_allocation() {
        let encoded = compressed(&[0; 256]);
        let mut remaining = 16;
        assert!(inflate(ZlibDecoder::new(&encoded[8..]), &mut remaining).is_err());
    }
    #[test]
    /// Charge repeated aliases for each materialized leaf rather than only the backing stream.
    fn duplicate_external_references_have_a_separate_copy_budget() {
        let streams = vec![vec![7; 32]];
        let mut remaining = 1024;
        let mut p = Parser { version: 1600, streams: &streams, remaining: &mut remaining, copied: 40, visited: 1024, count: 0 };
        let reference: [u32; 4] = [0, 32, 0, 0];
        let body: Vec<u8> = reference.into_iter().flat_map(u32::to_le_bytes).collect();
        let mut chunks = chunk(b"data", &body);
        chunks.extend(chunk(b"data", &body));
        assert!(p.chunks(&chunks, None, 0).unwrap_err().contains("materialized"));
    }

    /// Real modern external LIST aliases preserve PowerClip artwork under both page transforms.
    #[test]
    fn external_powerclip_frame_and_resource_lists_import_every_occurrence() {
        use vectorcraft_geom::Shape;

        /// Encode generated object arguments ahead of their offset and reversed-type tables.
        fn loda(kind: u32, args: &[(u32, Vec<u8>)]) -> Vec<u8> {
            let count = args.len() as u32;
            let table = 20 + args.iter().map(|(_, data)| data.len() as u32).sum::<u32>();
            let mut data: Vec<u8> = [table + count * 8, count, table, table + count * 4, kind].into_iter().flat_map(u32::to_le_bytes).collect();
            let mut offsets = Vec::new();
            for (_, value) in args {
                offsets.push(data.len() as u32);
                data.extend(value);
            }
            data.extend(offsets.into_iter().flat_map(u32::to_le_bytes));
            data.extend(args.iter().rev().flat_map(|(id, _)| id.to_le_bytes()));
            chunk(b"loda", &data)
        }
        /// A triangle uses the same native curve layout in legacy and modern versions.
        fn triangle() -> Vec<u8> {
            let mut data: Vec<u8> = [3u32, 0, 0, 254000, 0, 0, 254000].into_iter().flat_map(u32::to_le_bytes).collect();
            data.extend([0, 64, 72]);
            data
        }

        let mut frame = b"obj ".to_vec();
        frame.extend(chunk(b"spnd", &10u32.to_le_bytes()));
        frame.extend(loda(
            3,
            &[
                (30, triangle()),
                (8005, [8u32, 1, 0, 0].into_iter().flat_map(u32::to_le_bytes).collect()),
                (1000, "Alias".encode_utf16().flat_map(u16::to_le_bytes).collect()),
            ],
        ));
        let mut page = b"page".to_vec();
        page.extend(chunk(b"LIST", &frame));
        let mut content = b"obj ".to_vec();
        content.extend(loda(3, &[(30, triangle())]));
        let mut group = b"grp ".to_vec();
        group.extend(chunk(b"spnd", &8u32.to_le_bytes()));
        group.extend(loda(0, &[(3500, 10u32.to_le_bytes().to_vec())]));
        group.extend(chunk(b"LIST", &content));
        let mut resources = b"clpt".to_vec();
        resources.extend(chunk(b"LIST", &group));
        let streams = vec![resources, page];
        let mut root = Vec::new();
        for (index, stream) in streams.iter().enumerate() {
            let reference: Vec<u8> = [index as u32, stream.len() as u32, 0, 0].into_iter().flat_map(u32::to_le_bytes).collect();
            for _ in 0..2 {
                root.extend(chunk(b"LIST", &reference));
            }
        }
        let mut remaining = MAX_BYTES;
        let mut parser = Parser { version: 1600, streams: &streams, remaining: &mut remaining, copied: MAX_BYTES, visited: MAX_BYTES * 4, count: 0 };
        let chunks = parser.chunks(&root, None, 0).unwrap();
        assert_eq!(chunks[0], chunks[1]);
        assert_eq!(chunks[2], chunks[3]);
        assert!(!std::ptr::eq(&chunks[2].children[0], &chunks[3].children[0]));
        let imported = crate::drawing::import(&chunks, 1600).unwrap();
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
        assert_eq!(imported.document.artboards.len(), 2);
        assert_eq!(imported.document.layers.len(), 2);
        let offset = imported.document.artboards[1].rect.x0;
        for (index, layer) in imported.document.layers.iter().enumerate() {
            let nodes = layer.children().unwrap();
            assert_eq!(nodes.len(), 1);
            let clip = &nodes[0];
            assert!(clip.clips());
            assert_eq!(clip.name.as_deref(), Some("Alias"));
            let contents = clip.children().unwrap();
            assert_eq!(contents.len(), 2);
            assert_eq!(contents[0].name, None, "the wrapper takes the name rather than cloning it");
            for node in contents {
                let bounds = node.path_data().unwrap().to_bezpath().bounding_box();
                assert!((bounds.x0 - 306.0 - index as f64 * offset).abs() < 1e-8);
            }
        }
    }

    #[test]
    /// Reject nonterminal zero records and overflowing version-record lengths.
    fn zero_records_and_huge_version_lengths_are_rejected() {
        let mut zeros = vec![0; 65536];
        zeros.extend_from_slice(b"nonzero!");
        assert!(load(&riff(&zeros)).is_err());
        let mut fver = b"fver".to_vec();
        fver.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(load(&riff(&fver)).is_err());
    }
    #[test]
    /// Bound repeated external LIST traversal independently of leaf-copy allocations.
    fn repeated_external_lists_charge_the_traversal_budget() {
        let mut list = b"data".to_vec();
        list.extend_from_slice(&[0; 32]);
        let streams = vec![list];
        let mut remaining = 1024;
        let mut p = Parser { version: 1600, streams: &streams, remaining: &mut remaining, copied: 1024, visited: 100, count: 0 };
        let body: Vec<u8> = [0u32, 36, 0, 0].into_iter().flat_map(u32::to_le_bytes).collect();
        let mut chunks = chunk(b"LIST", &body);
        chunks.extend(chunk(b"LIST", &body));
        assert!(p.chunks(&chunks, None, 0).unwrap_err().contains("traversal"));
    }
}
