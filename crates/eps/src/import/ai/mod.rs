//! Illustrator's editing data: the document as the app keeps it, which an Illustrator EPS carries
//! after its printed page and a `.ai` file (PDF compatible or not) in its first page's
//! `/PieceInfo /Illustrator /Private` streams.
//!
//! - In an EPS it starts at the `%AI9_PrivateDataBegin` comment: header comments, then
//!   `%AI24_DataStream` (Zstandard) or `%AI9_DataStream` (zlib) and the compressed data as ASCII85
//!   on comment lines, up to `%AI9_PrivateDataEnd`. Data that isn't compressed follows the header
//!   as it is.
//! - In a `.ai` file the streams joined start with `%AI24_ZStandard_Data` (Zstandard) or
//!   `%AI12_CompressedData` (zlib), else are the data itself ([`decode_private`]).
//!
//! The data is a PostScript-like program of the legacy Illustrator format's operators (layers,
//! groups, paths, colours, gradients, images) with newer ones (transparency, hidden objects) and
//! dictionaries (names, artboards, type). `lex` scans it, `read` builds the document.
//!
//! Reading is bounded: the data decompresses to at most [`MAX_DATA`], and the reader caps nesting,
//! objects and points. Anything it doesn't read is an error: the caller opens the file the way it
//! did before (the printed page, or the PDF), with a warning saying why.

mod lex;
mod obj;
mod paint;
mod read;

use std::io::Read as _;

use super::lex::find;
use crate::ps;

pub use read::{Structure, read};
pub(crate) use read::{slot_frame, slot_of};

#[cfg(test)]
mod tests;

/// Largest editing data read, decompressed (bytes).
pub const MAX_DATA: u64 = 256 << 20;
/// Largest Zstandard window accepted (a frame asking for more is damaged data).
const MAX_ZSTD_WINDOW: u64 = 64 << 20;
/// Most Zstandard frames read (the app writes one).
const MAX_FRAMES: usize = 4096;

const EPS_BEGIN: &[u8] = b"%AI9_PrivateDataBegin";
const EPS_END: &[u8] = b"%AI9_PrivateDataEnd";

/// The editing data an Illustrator EPS carries (`ps`: its PostScript), decoded: `None` without
/// one, an error when it can't be decoded.
pub fn eps_data(ps: &[u8]) -> Option<Result<Vec<u8>, String>> {
    let start = find(ps, EPS_BEGIN)? + EPS_BEGIN.len();
    let rest = ps.get(start..)?;
    let section = rest.get(..find(rest, EPS_END).unwrap_or(rest.len())).unwrap_or_default();
    Some(eps_section(section))
}

/// The editing data between the EPS markers: header comments, then the data (encoded or not).
fn eps_section(section: &[u8]) -> Result<Vec<u8>, String> {
    let mut header = Vec::new();
    let mut lines = section.split(|b| matches!(b, b'\n' | b'\r')).filter(|l| !l.is_empty());
    let mut codec = None;
    for line in lines.by_ref() {
        if line.starts_with(b"%AI24_DataStream") {
            codec = Some(Codec::Zstd);
            break;
        }
        if line.starts_with(b"%AI9_DataStream") {
            codec = Some(Codec::Zlib);
            break;
        }
        header.extend_from_slice(line);
        header.extend_from_slice(b"\n");
        if header.len() as u64 > MAX_DATA {
            return Err(too_large());
        }
    }
    let Some(codec) = codec else {
        // Not compressed: the section is the data, as it is (images' samples are binary).
        if section.len() as u64 > MAX_DATA {
            return Err(too_large());
        }
        return Ok(section.to_vec());
    };
    let mut text = String::new();
    for line in lines {
        let body = line.strip_prefix(b"%").unwrap_or(line);
        text.push_str(std::str::from_utf8(body).map_err(|_| damaged("its ASCII85 encoding"))?);
        if text.len() as u64 > MAX_DATA {
            return Err(too_large());
        }
    }
    if !text.contains("~>") {
        text.push_str("~>");
    }
    let packed = ps::ascii85_decode(&text).ok_or_else(|| damaged("its ASCII85 encoding"))?;
    let mut data = header;
    data.extend(codec.decode(&packed, MAX_DATA.saturating_sub(data.len() as u64))?);
    Ok(data)
}

/// The editing data of a `.ai` file: its `AIPrivateData` streams joined (`raw`), decompressed. The
/// compressed data may follow header comments that aren't.
pub fn decode_private(raw: &[u8]) -> Result<Vec<u8>, String> {
    if raw.len() as u64 > MAX_DATA {
        return Err(too_large());
    }
    for (marker, codec) in [(&b"%AI24_ZStandard_Data"[..], Codec::Zstd), (b"%AI12_CompressedData", Codec::Zlib)] {
        if let Some(at) = find(raw, marker) {
            let (header, body) = raw.split_at(at);
            let mut data = header.to_vec();
            data.extend(codec.decode(body.get(marker.len()..).unwrap_or_default(), MAX_DATA.saturating_sub(header.len() as u64))?);
            return Ok(data);
        }
    }
    Ok(raw.to_vec())
}

#[derive(Clone, Copy)]
enum Codec {
    Zstd,
    Zlib,
}

impl Codec {
    /// `data` decompressed, at most `max` bytes.
    fn decode(self, data: &[u8], max: u64) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        match self {
            Codec::Zlib => {
                flate2::read::ZlibDecoder::new(data).take(max + 1).read_to_end(&mut out).map_err(|_| damaged("its zlib compression"))?;
            }
            Codec::Zstd => {
                // The data may be several frames one after the other, padded with zeros.
                let end = data.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
                let mut src = data.get(..end).unwrap_or_default();
                let mut frames = 0usize;
                while !src.is_empty() {
                    frames += 1;
                    if frames > MAX_FRAMES {
                        return Err(damaged("its Zstandard compression"));
                    }
                    let left = (max + 1).saturating_sub(out.len() as u64);
                    if left == 0 {
                        break;
                    }
                    let d = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(&mut src, MAX_ZSTD_WINDOW)
                        .map_err(|_| damaged("its Zstandard compression"))?;
                    d.take(left).read_to_end(&mut out).map_err(|_| damaged("its Zstandard compression"))?;
                }
            }
        }
        if out.len() as u64 > max {
            return Err(too_large());
        }
        Ok(out)
    }
}

fn damaged(what: &str) -> String {
    format!("{what} is damaged")
}

fn too_large() -> String {
    format!("it decompresses to more than {} MB", MAX_DATA >> 20)
}
