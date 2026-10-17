//! A store-only (uncompressed) ZIP archive: several exported files as one download. The files are
//! already compressed (PNG, JPEG, PDF…), so storing them keeps the archive fast to write and read.
//!
//! [`Archive`] reads the files of a ZIP archive, stored or deflated, as PKWARE's APPNOTE.TXT
//! describes the format (<https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT>).

use std::io::Read as _;

use flate2::Crc;

/// The DOS date of every entry: 1980-01-01, the earliest a ZIP date can say (no clock on the web,
/// and a fixed date keeps archives of the same files identical).
const DOS_DATE: u16 = (1 << 5) | 1;
/// General purpose flag: file names are UTF-8.
const UTF8_NAMES: u16 = 1 << 11;
/// Version needed to extract: 2.0.
const VERSION: u16 = 20;

/// The archive of `files` (`(name, bytes)`; `/` separates folders in a name), stored without
/// compression. Fails past the limits of a ZIP without its 64-bit extension (65535 files, 4 GB).
pub fn store<N: AsRef<str>, B: AsRef<[u8]>>(files: &[(N, B)]) -> Result<Vec<u8>, String> {
    let too_big = || "too much to zip: at most 65535 files and 4 GB".to_string();
    let count = u16::try_from(files.len()).map_err(|_| too_big())?;
    let mut out = Vec::with_capacity(files.iter().map(|(n, b)| 76 + 2 * n.as_ref().len() + b.as_ref().len()).sum::<usize>() + 22);
    let mut central = Vec::new();
    for (name, bytes) in files {
        let (name, bytes) = (name.as_ref().as_bytes(), bytes.as_ref());
        let offset = u32::try_from(out.len()).map_err(|_| too_big())?;
        let size = u32::try_from(bytes.len()).map_err(|_| too_big())?;
        let name_len = u16::try_from(name.len()).map_err(|_| format!("file name too long to zip: {}", String::from_utf8_lossy(name)))?;
        let mut crc = Crc::new();
        crc.update(bytes);
        // The fields both headers share: flags, method (0: stored), time, date, CRC, sizes, name length.
        let mut shared = Vec::with_capacity(26);
        for v in [UTF8_NAMES, 0, 0, DOS_DATE] {
            shared.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc.sum(), size, size] {
            shared.extend_from_slice(&v.to_le_bytes());
        }
        shared.extend_from_slice(&name_len.to_le_bytes());
        // Local file header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&shared);
        out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        out.extend_from_slice(name);
        out.extend_from_slice(bytes);
        // Central directory record.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&VERSION.to_le_bytes()); // made by
        central.extend_from_slice(&VERSION.to_le_bytes()); // needed to extract
        central.extend_from_slice(&shared);
        // Extra and comment lengths, disk number, internal and external attributes.
        central.extend_from_slice(&[0; 2 + 2 + 2 + 2 + 4]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let start = u32::try_from(out.len()).map_err(|_| too_big())?;
    let central_len = u32::try_from(central.len()).map_err(|_| too_big())?;
    out.extend_from_slice(&central);
    // End of central directory.
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // this disk, the directory's disk
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    Ok(out)
}

/// Does `bytes` start with a ZIP archive's first local file header?
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(&0x0403_4b50u32.to_le_bytes())
}

/// Why [`Archive`] can't read an archive or one of its files.
#[derive(Debug, PartialEq)]
pub enum ReadError {
    /// Not a ZIP archive: no end of central directory record.
    NotZip,
    /// More files than the reader was asked to accept.
    TooManyFiles,
    /// The archive needs the 64-bit (ZIP64) records, which the reader does not read.
    Zip64,
    /// The file is encrypted.
    Encrypted(String),
    /// The file is compressed with a method other than stored (0) or deflated (8).
    Method(String, u16),
    /// The file unpacks to more bytes than the reader was asked to accept.
    TooLarge(String),
    /// The archive (empty name) or the file is truncated, inconsistent or fails its CRC.
    Damaged(String),
}

/// One file of an archive, as its central directory record describes it.
struct Entry {
    name: String,
    flags: u16,
    method: u16,
    crc: u32,
    packed: usize,
    size: usize,
    local: usize,
}

/// A ZIP archive's files, listed from its central directory.
pub struct Archive<'a> {
    zip: &'a [u8],
    entries: Vec<Entry>,
}

impl<'a> Archive<'a> {
    /// The archive `zip`, with at most `max_files` files.
    pub fn open(zip: &'a [u8], max_files: usize) -> Result<Self, ReadError> {
        let damaged = || ReadError::Damaged(String::new());
        let u16_at = |at: usize| at.checked_add(2).and_then(|end| zip.get(at..end)).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
        let u32_at = |at: usize| at.checked_add(4).and_then(|end| zip.get(at..end)).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
        // The end record is 22 bytes followed by a comment of at most 65535 bytes: the signature
        // whose comment ends the archive, else the last signature (the archive has bytes after it).
        let last = zip.len().checked_sub(22).ok_or(ReadError::NotZip)?;
        let candidates = || (last.saturating_sub(0xFFFF)..=last).rev().filter(|&at| u32_at(at) == Some(0x0605_4b50));
        let end =
            candidates().find(|&at| u16_at(at + 20).map(usize::from) == Some(last - at)).or_else(|| candidates().next()).ok_or(ReadError::NotZip)?;
        let count = u16_at(end + 10).ok_or_else(damaged)?;
        let start = u32_at(end + 16).ok_or_else(damaged)?;
        if count == 0xFFFF || start == 0xFFFF_FFFF || u32_at(end + 12) == Some(0xFFFF_FFFF) {
            return Err(ReadError::Zip64);
        }
        let count = usize::from(count);
        if count > max_files {
            return Err(ReadError::TooManyFiles);
        }
        let mut at = start as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32_at(at) != Some(0x0201_4b50) {
                return Err(damaged());
            }
            let short = |off: usize| at.checked_add(off).and_then(u16_at).ok_or_else(damaged);
            let long = |off: usize| at.checked_add(off).and_then(u32_at).ok_or_else(damaged);
            let (name_len, extra, comment) = (usize::from(short(28)?), usize::from(short(30)?), usize::from(short(32)?));
            let name_at = at.checked_add(46).ok_or_else(damaged)?;
            let name = name_at.checked_add(name_len).and_then(|end| zip.get(name_at..end)).ok_or_else(damaged)?;
            entries.push(Entry {
                name: String::from_utf8_lossy(name).into_owned(),
                flags: short(8)?,
                method: short(10)?,
                crc: long(16)?,
                packed: long(20)? as usize,
                size: long(24)? as usize,
                local: long(42)? as usize,
            });
            at = name_at.checked_add(name_len + extra + comment).ok_or_else(damaged)?;
        }
        Ok(Self { zip, entries })
    }

    /// The file `name`, matched in any ASCII case.
    fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Does the archive hold the file `name` (in any ASCII case)?
    pub fn contains(&self, name: &str) -> bool {
        self.entry(name).is_some()
    }

    /// The bytes of the file `name` (matched in any ASCII case; None: the archive has no such
    /// file), when it unpacks to at most `max` bytes and passes its CRC.
    pub fn read(&self, name: &str, max: usize) -> Result<Option<Vec<u8>>, ReadError> {
        let Some(e) = self.entry(name) else { return Ok(None) };
        let damaged = || ReadError::Damaged(e.name.clone());
        if e.flags & 1 != 0 {
            return Err(ReadError::Encrypted(e.name.clone()));
        }
        if e.size > max {
            return Err(ReadError::TooLarge(e.name.clone()));
        }
        // The local header: its signature, then its name and extra field lengths at 26 and 28.
        let header = e.local.checked_add(30).and_then(|end| self.zip.get(e.local..end)).ok_or_else(damaged)?;
        let (Some(sig), Some(name_len), Some(extra)) = (header.get(..4), header.get(26..28), header.get(28..30)) else { return Err(damaged()) };
        if sig != 0x0403_4b50u32.to_le_bytes() {
            return Err(damaged());
        }
        let len = |b: &[u8]| b.try_into().map(|b| usize::from(u16::from_le_bytes(b))).map_err(|_| damaged());
        let start = e.local + 30 + len(name_len)? + len(extra)?;
        let data = start.checked_add(e.packed).and_then(|end| self.zip.get(start..end)).ok_or_else(damaged)?;
        let bytes = match e.method {
            0 if e.packed != e.size => return Err(damaged()),
            0 => data.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(e.size);
                flate2::read::DeflateDecoder::new(data).take((max as u64).saturating_add(1)).read_to_end(&mut out).map_err(|_| damaged())?;
                if out.len() > max {
                    return Err(ReadError::TooLarge(e.name.clone()));
                }
                out
            }
            m => return Err(ReadError::Method(e.name.clone(), m)),
        };
        let mut crc = Crc::new();
        crc.update(&bytes);
        if bytes.len() != e.size || crc.sum() != e.crc {
            return Err(damaged());
        }
        Ok(Some(bytes))
    }
}

/// The entries of a store-only archive written by [`store`] (read through its central directory,
/// checking every CRC) → `(name, bytes)`.
#[cfg(test)]
pub fn entries(zip: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |at: usize| zip.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or("truncated");
    let u32_at = |at: usize| zip.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("truncated");
    let end = zip.len().checked_sub(22).ok_or("no end record")?;
    if u32_at(end)? != 0x0605_4b50 {
        return Err("no end record".into());
    }
    let count = u16_at(end + 10)? as usize;
    let mut at = u32_at(end + 16)? as usize;
    let mut out = vec![];
    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 || u16_at(at + 10)? != 0 {
            return Err("not a stored central record".into());
        }
        let (crc, size, name_len) = (u32_at(at + 16)?, u32_at(at + 20)? as usize, u16_at(at + 28)? as usize);
        let extra = u16_at(at + 30)? as usize + u16_at(at + 32)? as usize;
        let local = u32_at(at + 42)? as usize;
        let name = String::from_utf8(zip.get(at + 46..at + 46 + name_len).ok_or("truncated")?.to_vec()).map_err(|e| e.to_string())?;
        if u32_at(local)? != 0x0403_4b50 {
            return Err(format!("{name}: no local header"));
        }
        let data = local + 30 + u16_at(local + 26)? as usize + u16_at(local + 28)? as usize;
        let bytes = zip.get(data..data + size).ok_or("truncated")?.to_vec();
        let mut c = Crc::new();
        c.update(&bytes);
        if c.sum() != crc {
            return Err(format!("{name}: CRC mismatch"));
        }
        out.push((name, bytes));
        at += 46 + name_len + extra;
    }
    Ok(out)
}
