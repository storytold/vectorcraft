//! Illustrator's editing data: the document as the app keeps it, which an Illustrator EPS carries
//! after its printed page and a `.ai` file (PDF compatible or not) in its first page's
//! `/PieceInfo /Illustrator /Private` streams.
//!
//! - In an EPS it starts at the `%AI9_PrivateDataBegin` comment: header comments, then
//!   `%AI24_DataStream` (Zstandard) or `%AI9_DataStream` (zlib) and the compressed data as ASCII85
//!   on comment lines, up to `%AI9_PrivateDataEnd`. Data that isn't compressed follows the header
//!   as it is.
//! - In a `.ai` file the streams, read one after the other, start with `%AI24_ZStandard_Data`
//!   (Zstandard) or `%AI12_CompressedData` (zlib), else are the data itself ([`decode_private_from`]).
//!
//! The data is a PostScript-like program of the legacy Illustrator format's operators (layers,
//! groups, paths, colours, gradients, images) with newer ones (transparency, hidden objects) and
//! dictionaries (names, artboards, type). `lex` scans it, `read` builds the document.
//!
//! Reading is bounded: the data decompresses to at most [`max_data`], and the reader caps nesting,
//! objects and points. Anything it doesn't read is an error: the caller opens the file the way it
//! did before (the printed page, or the PDF), with a warning saying why.

mod arrowhead;
mod blend;
mod brush;
mod envelope;
mod frames;
mod lex;
mod mesh;
mod obj;
mod paint;
mod palette;
mod read;
mod style;

use std::io::Read as _;

use super::lex::find;
use crate::ps;

#[cfg(any(test, feature = "work-count"))]
pub use blend::blend_work;
pub(crate) use blend::{MAX_BLEND_OBJECTS, MAX_KEYS as MAX_BLEND_KEYS, PLUGIN as BLEND_PLUGIN, blend_reads};
pub(crate) use brush::{
    BrushesRead, MANAGER_NAME as BRUSH_MANAGER, brushes_kept, plugin_name as brush_plugin_name, read_brush_object, turn_over as turn_brush_art_over,
};
#[cfg(test)]
pub(crate) use envelope::DASH_CHECKS_MADE;
pub(crate) use envelope::DRAWN as ENVELOPE_DRAWN;
pub(crate) use envelope::TYPE_DRAWN as ENVELOPE_TYPE_DRAWN;
pub(crate) use envelope::{GridPoint as EnvelopeGridPoint, PLUGIN as DEFORM_PLUGIN, outline as envelope_outline, upright as envelope_upright};
pub(crate) use envelope::{MAX_ANY as ENVELOPE_MAX_ANY, MAX_SOLID as ENVELOPE_MAX_SOLID};
pub(crate) use envelope::{MAX_CHECKED, MAX_DASH_CHECKS};
pub(crate) use envelope::{WARPED_TYPE_DRAWN, WarpedType};
#[cfg(test)]
pub(crate) use frames::TEXT_FRAME_SCANS;
pub(crate) use frames::text_frames;
pub(crate) use mesh::{MAX_SIDE as MAX_MESH_SIDE, mesh_reads};
pub use obj::{NameEncoding, Names};
pub(crate) use paint::MAX_STOPS as MAX_GRADIENT_STOPS;
#[cfg(test)]
pub(crate) use read::objects_read;
#[cfg(test)]
pub use read::read;
#[cfg(test)]
pub(crate) use read::with_image_limits;
pub(crate) use read::{
    BLEND_BUDGET, LINKED_PREVIEWS, MAX_NODES as MAX_EDITING_OBJECTS, MAX_SYMBOLS, ParsedStyle, join_and, parse_art_style, parsed_moves_with,
    parsed_type_color, read_over_page, read_parsed_style, read_program, read_style_list, slot_frame, slot_of, symbol_art_member,
};
pub use read::{
    BindingRef, Brushes, PaletteSwatches, PlacedEps, PlacedPage, Structure, Symbols, active_artboard, artboards, bindings_note, brushes,
    graphic_styles, palette, read_beside, read_in, symbols, variables,
};
#[cfg(test)]
pub(crate) use style::ART_STYLE_PARSES;

pub(crate) use read::{clip_by_type, is_look_slot};
pub(crate) use style::{MAX_LISTED, MAX_LISTED_NAME, warp_number};

#[cfg(test)]
mod tests;

/// Least editing data read, decompressed (bytes), whatever the machine: 512 MiB on 64-bit builds (a
/// 30 MB poker box template with embedded rasters unpacks to 322 MiB), 256 MiB in a 32-bit address
/// space (wasm).
pub const MIN_DATA: u64 = if cfg!(target_pointer_width = "64") { 512 << 20 } else { 256 << 20 };
/// Most editing data read on any machine (bytes). Reading takes about as much memory again as the
/// data (1.1 GB of data with 24 embedded 4096 x 4096 images peaked 1.1 GB above the start).
pub const MAX_DATA: u64 = if cfg!(target_pointer_width = "64") { 2 << 30 } else { 256 << 20 };

/// Largest editing data read, decompressed (bytes): an eighth of installed memory, from
/// [`MIN_DATA`] to [`MAX_DATA`], so a file whose embedded rasters unpack past 512 MiB keeps its
/// layers on a machine with room for it (#3510), while a small file that unpacks without end
/// still stops early.
pub fn max_data() -> u64 {
    static LIMIT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| data_limit(vectorcraft_render::installed_memory()))
}

/// [`max_data`] on a machine with `memory` bytes installed (`None`: not known).
fn data_limit(memory: Option<u64>) -> u64 {
    memory.map_or(MIN_DATA, |m| (m / 8).clamp(MIN_DATA, MAX_DATA))
}
/// Largest Zstandard window accepted (a frame asking for more is damaged data).
const MAX_ZSTD_WINDOW: u64 = 64 << 20;
/// Most Zstandard frames read (the app writes one).
const MAX_FRAMES: usize = 4096;

#[cfg(test)]
thread_local! {
    /// How many bytes [`Codec::decode`] has looked at on this thread to find whether another
    /// Zstandard frame follows or only zero padding is left.
    static PADDING_LOOKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// [`PADDING_LOOKS`] of this thread.
#[cfg(test)]
fn padding_looks() -> u64 {
    PADDING_LOOKS.get()
}

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

/// Does the EPS `ps` carry editing data ([`eps_data`] isn't `None`)?
pub fn has_eps_data(ps: &[u8]) -> bool {
    find(ps, EPS_BEGIN).is_some()
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
        if header.len() as u64 > max_data() {
            return Err(too_large());
        }
    }
    let Some(codec) = codec else {
        // Not compressed: the section is the data, as it is (images' samples are binary).
        if section.len() as u64 > max_data() {
            return Err(too_large());
        }
        return Ok(section.to_vec());
    };
    let mut text = String::new();
    for line in lines {
        let body = line.strip_prefix(b"%").unwrap_or(line);
        text.push_str(std::str::from_utf8(body).map_err(|_| damaged("its ASCII85 encoding"))?);
        if text.len() as u64 > max_data() {
            return Err(too_large());
        }
    }
    if !text.contains("~>") {
        text.push_str("~>");
    }
    let packed = ps::ascii85_decode(&text).ok_or_else(|| damaged("its ASCII85 encoding"))?;
    let mut data = header;
    data.extend(codec.decode(packed.as_slice(), max_data().saturating_sub(data.len() as u64))?);
    Ok(data)
}

/// The markers the compressed editing data of a `.ai` file follows, with its compression.
const PRIVATE_MARKERS: [(&[u8], Codec); 2] = [(b"%AI24_ZStandard_Data", Codec::Zstd), (b"%AI12_CompressedData", Codec::Zlib)];

/// The editing data of a `.ai` file: its `AIPrivateData` streams joined (`raw`), decompressed, as
/// [`decode_private_from`] reads them.
pub fn decode_private(raw: &[u8]) -> Result<Vec<u8>, String> {
    decode_private_from(raw)
}

/// The editing data of a `.ai` file, read from its `AIPrivateData` streams one after the other
/// (`raw`) and decompressed as it is read: the streams are never joined in memory. The compressed
/// data follows the first [`PRIVATE_MARKERS`] marker, which may follow header comments that aren't
/// compressed; without a marker the streams are the data itself. The data decompresses to at most
/// [`max_data`] bytes, and at most [`max_encoded`] bytes are read from `raw`.
pub fn decode_private_from(raw: impl std::io::Read) -> Result<Vec<u8>, String> {
    decode_private_within(raw, max_encoded())
}

/// [`decode_private_from`], reading at most `most` bytes of `raw`.
fn decode_private_within(raw: impl std::io::Read, most: u64) -> Result<Vec<u8>, String> {
    let mut counted = Counted { inner: raw.take(most + 1), read: 0 };
    let data = private_from(&mut counted);
    if counted.read > most {
        return Err(too_large());
    }
    data
}

/// Most bytes of a `.ai` file's editing data streams read ([`decode_private_from`]): [`max_data`]
/// and a sixteenth more. Compressed data that decompresses to at most [`max_data`] bytes takes no
/// more than that: the compression's own block headers add a few bytes in 64 KiB, and only zero
/// padding or blocks that hold nothing would go past it.
fn max_encoded() -> u64 {
    max_data().saturating_add(max_data() >> 4)
}

/// [`decode_private_from`] of `raw`, with no limit on what it reads of `raw`.
fn private_from(raw: impl std::io::Read) -> Result<Vec<u8>, String> {
    use std::io::BufRead as _;
    let longest = PRIVATE_MARKERS.iter().map(|(m, _)| m.len()).max().unwrap_or(0);
    let mut src = std::io::BufReader::with_capacity(64 << 10, raw);
    let mut data = Vec::new();
    loop {
        let buf = src.fill_buf().map_err(|e| e.to_string())?;
        if buf.is_empty() {
            return Ok(data);
        }
        let (read, from) = (buf.len(), data.len().saturating_sub(longest - 1));
        data.extend_from_slice(buf);
        src.consume(read);
        let tail = data.get(from..).unwrap_or_default();
        let marker = PRIVATE_MARKERS.iter().filter_map(|&(m, codec)| Some((from + find(tail, m)?, m.len(), codec))).min_by_key(|&(at, ..)| at);
        if let Some((at, len, codec)) = marker {
            if at as u64 > max_data() {
                return Err(too_large());
            }
            let body = data.split_off(at + len);
            data.truncate(at);
            data.extend(codec.decode(std::io::Cursor::new(body).chain(src), max_data() - at as u64)?);
            return Ok(data);
        }
        if data.len() as u64 > max_data() {
            return Err(too_large());
        }
    }
}

/// A reader that counts the bytes read from `inner`.
struct Counted<R> {
    inner: R,
    read: u64,
}

impl<R: std::io::Read> std::io::Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read = self.read.saturating_add(n as u64);
        Ok(n)
    }
}

#[derive(Clone, Copy)]
enum Codec {
    Zstd,
    Zlib,
}

impl Codec {
    /// `data` decompressed, at most `max` bytes.
    fn decode(self, data: impl std::io::Read, max: u64) -> Result<Vec<u8>, String> {
        use std::io::BufRead as _;
        let mut out = Vec::new();
        match self {
            Codec::Zlib => {
                flate2::read::ZlibDecoder::new(data).take(max + 1).read_to_end(&mut out).map_err(|_| damaged("its zlib compression"))?;
            }
            Codec::Zstd => {
                // The data may be several frames one after the other, padded with zeros. A frame
                // can itself end in a zero byte, so the padding is what is left after the last frame.
                let zstd_damaged = || damaged("its Zstandard compression");
                let mut src = std::io::BufReader::new(data);
                let mut frames = 0usize;
                loop {
                    // Zeros before a frame aren't padding: the frame that would start there is damaged.
                    let mut after_zeros = false;
                    loop {
                        let buf = src.fill_buf().map_err(|_| zstd_damaged())?;
                        let Some(&first) = buf.first() else { break };
                        #[cfg(test)]
                        PADDING_LOOKS.set(PADDING_LOOKS.get().saturating_add(1));
                        if first != 0 {
                            break;
                        }
                        let zeros = buf.iter().position(|b| *b != 0);
                        #[cfg(test)]
                        PADDING_LOOKS.set(PADDING_LOOKS.get().saturating_add(zeros.unwrap_or(buf.len()) as u64));
                        // (Zeros that end a buffer may go on in the next one, or end the data.)
                        after_zeros = true;
                        let n = zeros.unwrap_or(buf.len());
                        src.consume(n);
                    }
                    if src.fill_buf().map_err(|_| zstd_damaged())?.is_empty() {
                        break;
                    }
                    frames += 1;
                    if frames > MAX_FRAMES {
                        return Err(zstd_damaged());
                    }
                    let left = (max + 1).saturating_sub(out.len() as u64);
                    if left == 0 {
                        break;
                    }
                    if after_zeros {
                        return Err(zstd_damaged());
                    }
                    let d = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(&mut src, MAX_ZSTD_WINDOW).map_err(|_| zstd_damaged())?;
                    d.take(left).read_to_end(&mut out).map_err(|_| zstd_damaged())?;
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
    format!("it decompresses to more than {} MB", max_data() >> 20)
}
