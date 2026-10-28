//! Detection and container-level navigation for CorelDRAW files.
//! Supports both legacy binary RIFF containers (v3 to v13/X3) and modern ZIP containers (X4+).

use crate::error::Error;

/// The container format of a CorelDRAW document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerType {
    /// RIFF-based container (CorelDRAW 3 to 13 / X3).
    Riff { fourcc: [u8; 4], version_num: Option<u16> },
    /// PKZIP-based archive (CorelDRAW X4 / v14 onwards).
    Zip,
}

/// Check if bytes appear to be a CorelDRAW document (.cdr).
pub fn is_cdr(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    // 1. Classic RIFF container: starts with RIFF, form type begins with CDR or CDRX
    if bytes.starts_with(b"RIFF") && bytes.get(8..12).is_some_and(|form| form.starts_with(b"CDR")) {
        return true;
    }
    // 2. Modern ZIP-based container: starts with PK\x03\x04
    if bytes.starts_with(b"PK\x03\x04") {
        // Quick scan within the first 64KB for Corel signature paths
        let limit = bytes.len().min(64 * 1024);
        if let Some(slice) = bytes.get(..limit) {
            let is_corel = contains_subslice(slice, b"content/riffData.cdr")
                || contains_subslice(slice, b"metadata/thumbnails/thumbnail.png")
                || contains_subslice(slice, b"content/root.dat")
                || contains_subslice(slice, b"application/vnd.corel-draw")
                || contains_subslice(slice, b"application/x-coreldraw");
            if is_corel {
                return true;
            }
        }
    }
    false
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// A parsed RIFF chunk header.
#[derive(Debug, Clone, Copy)]
pub struct RiffChunk<'a> {
    pub id: [u8; 4],
    pub data: &'a [u8],
}

/// Read all top-level chunks of a RIFF container (after the initial 12-byte RIFF header).
pub fn riff_chunks(bytes: &[u8]) -> Result<(&[u8; 4], Vec<RiffChunk<'_>>), Error> {
    if bytes.len() < 12 || !bytes.starts_with(b"RIFF") {
        return Err(Error::Malformed("not a RIFF container"));
    }
    let form_slice = bytes.get(8..12).ok_or(Error::Malformed("truncated RIFF header"))?;
    let form: &[u8; 4] = form_slice.try_into().map_err(|_| Error::Malformed("invalid form FourCC"))?;

    let file_len = u32::from_le_bytes(
        bytes.get(4..8).ok_or(Error::Malformed("missing RIFF size"))?.try_into().map_err(|_| Error::Malformed("invalid RIFF size"))?,
    ) as usize;

    let total = bytes.len().min(file_len.saturating_add(8));
    let mut chunks = Vec::new();
    let mut offset = 12;

    while offset + 8 <= total {
        let cid_bytes = bytes.get(offset..offset + 4).ok_or(Error::Malformed("unexpected EOF"))?;
        let cid: [u8; 4] = cid_bytes.try_into().map_err(|_| Error::Malformed("chunk id"))?;

        let csz = u32::from_le_bytes(
            bytes.get(offset + 4..offset + 8).ok_or(Error::Malformed("unexpected EOF"))?.try_into().map_err(|_| Error::Malformed("chunk size"))?,
        ) as usize;

        let data_start = offset + 8;
        let data_end = data_start.checked_add(csz).ok_or(Error::Limit("chunk size overflow"))?;
        let data = bytes.get(data_start..data_end.min(total)).ok_or(Error::Malformed("chunk truncated"))?;

        chunks.push(RiffChunk { id: cid, data });

        let advance = csz + (csz % 2); // 2-byte alignment
        offset = data_start.checked_add(advance).ok_or(Error::Limit("offset overflow"))?;
    }

    Ok((form, chunks))
}
