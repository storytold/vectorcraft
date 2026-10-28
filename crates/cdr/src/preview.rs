//! Thumbnail and preview extraction for CorelDRAW files.
//! Supports CF_DIB bitmaps in legacy RIFF files (chunk DISP) and PNG thumbnails in ZIP files.

use crate::container::{is_cdr, riff_chunks};
use crate::error::Error;
use flate2::read::DeflateDecoder;
use std::io::Read;

/// An extracted raster preview from a CorelDRAW document.
#[derive(Debug, Clone)]
pub struct Preview {
    /// Encoded image bytes (either standard BMP or PNG).
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Extract an embedded preview from a CorelDRAW document.
pub fn preview(bytes: &[u8]) -> Result<Preview, Error> {
    if !is_cdr(bytes) {
        return Err(Error::Malformed("not a CorelDRAW document"));
    }

    if bytes.starts_with(b"RIFF") {
        preview_from_riff(bytes)
    } else if bytes.starts_with(b"PK\x03\x04") {
        preview_from_zip(bytes)
    } else {
        Err(Error::Unsupported("unrecognized CorelDRAW container"))
    }
}

/// Extract thumbnail from legacy RIFF DISP chunk (CF_DIB format).
fn preview_from_riff(bytes: &[u8]) -> Result<Preview, Error> {
    let (_, chunks) = riff_chunks(bytes)?;
    let disp_chunk = chunks.iter().find(|c| &c.id == b"DISP").ok_or(Error::Unsupported("no DISP preview chunk"))?;

    let data = disp_chunk.data;
    if data.len() < 44 {
        return Err(Error::Malformed("truncated DISP chunk"));
    }

    let format = u32::from_le_bytes(
        data.get(0..4).ok_or(Error::Malformed("truncated DISP format"))?.try_into().map_err(|_| Error::Malformed("format integer"))?,
    );

    // Format 8 corresponds to CF_DIB (Windows Device Independent Bitmap)
    if format != 8 {
        return Err(Error::Unsupported("non-DIB preview in DISP chunk"));
    }

    let dib = data.get(4..).ok_or(Error::Malformed("missing DIB payload"))?;
    dib_to_bmp(dib)
}

/// Convert a CF_DIB stream (BITMAPINFOHEADER + palette + bits) into a standard BMP file.
fn dib_to_bmp(dib: &[u8]) -> Result<Preview, Error> {
    if dib.len() < 40 {
        return Err(Error::Malformed("truncated BITMAPINFOHEADER"));
    }

    let bi_size = u32::from_le_bytes(dib.get(0..4).ok_or(Error::Malformed("biSize"))?.try_into().map_err(|_| Error::Malformed("biSize"))?) as usize;
    let bi_width = i32::from_le_bytes(dib.get(4..8).ok_or(Error::Malformed("biWidth"))?.try_into().map_err(|_| Error::Malformed("biWidth"))?);
    let bi_height = i32::from_le_bytes(dib.get(8..12).ok_or(Error::Malformed("biHeight"))?.try_into().map_err(|_| Error::Malformed("biHeight"))?);
    let bi_bit_count =
        u16::from_le_bytes(dib.get(14..16).ok_or(Error::Malformed("biBitCount"))?.try_into().map_err(|_| Error::Malformed("biBitCount"))?);
    let bi_compression =
        u32::from_le_bytes(dib.get(16..20).ok_or(Error::Malformed("biCompression"))?.try_into().map_err(|_| Error::Malformed("biCompression"))?);
    let bi_clr_used =
        u32::from_le_bytes(dib.get(32..36).ok_or(Error::Malformed("biClrUsed"))?.try_into().map_err(|_| Error::Malformed("biClrUsed"))?) as usize;

    let width = bi_width.unsigned_abs();
    let height = bi_height.unsigned_abs();

    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(Error::Malformed("invalid image dimensions in preview"));
    }

    // Palette calculation
    let palette_bytes = if bi_bit_count <= 8 {
        let colors = if bi_clr_used > 0 && bi_clr_used <= (1 << bi_bit_count) { bi_clr_used } else { 1 << bi_bit_count };
        colors.checked_mul(4).ok_or(Error::Limit("palette size overflow"))?
    } else if bi_compression == 3 {
        // BI_BITFIELDS
        12
    } else {
        0
    };

    let offset = 14usize.checked_add(bi_size).and_then(|o| o.checked_add(palette_bytes)).ok_or(Error::Limit("offset overflow"))?;
    let file_size = 14usize.checked_add(dib.len()).ok_or(Error::Limit("file size overflow"))?;

    let mut bmp = Vec::with_capacity(file_size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes()); // Reserved 1
    bmp.extend_from_slice(&0u16.to_le_bytes()); // Reserved 2
    bmp.extend_from_slice(&(offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);

    Ok(Preview { data: bmp, width, height })
}

/// Extract PNG thumbnail from a modern ZIP-based CorelDRAW document.
fn preview_from_zip(bytes: &[u8]) -> Result<Preview, Error> {
    // Walk ZIP local file headers to find metadata/thumbnails/thumbnail.png or previews/thumbnail.png
    let mut offset = 0;
    while offset + 30 <= bytes.len() {
        if bytes.get(offset..offset + 4) != Some(b"PK\x03\x04") {
            break;
        }

        let method = u16::from_le_bytes(
            bytes.get(offset + 8..offset + 10).ok_or(Error::Malformed("method"))?.try_into().map_err(|_| Error::Malformed("method"))?,
        );
        let comp_size = u32::from_le_bytes(
            bytes.get(offset + 18..offset + 22).ok_or(Error::Malformed("comp_size"))?.try_into().map_err(|_| Error::Malformed("comp_size"))?,
        ) as usize;
        let name_len = u16::from_le_bytes(
            bytes.get(offset + 26..offset + 28).ok_or(Error::Malformed("name_len"))?.try_into().map_err(|_| Error::Malformed("name_len"))?,
        ) as usize;
        let extra_len = u16::from_le_bytes(
            bytes.get(offset + 28..offset + 30).ok_or(Error::Malformed("extra_len"))?.try_into().map_err(|_| Error::Malformed("extra_len"))?,
        ) as usize;

        let name_start = offset + 30;
        let name_end = name_start.checked_add(name_len).ok_or(Error::Limit("name overflow"))?;
        let name = bytes.get(name_start..name_end).ok_or(Error::Malformed("name truncated"))?;

        let data_start = name_end.checked_add(extra_len).ok_or(Error::Limit("data offset overflow"))?;
        let data_end = data_start.checked_add(comp_size).ok_or(Error::Limit("data size overflow"))?;

        if name == b"metadata/thumbnails/thumbnail.png" || name == b"previews/thumbnail.png" {
            let comp_data = bytes.get(data_start..data_end).ok_or(Error::Malformed("thumbnail data truncated"))?;
            let png_data = match method {
                0 => comp_data.to_vec(),
                8 => {
                    let mut decoder = DeflateDecoder::new(comp_data);
                    let mut decompressed = Vec::new();
                    decoder.read_to_end(&mut decompressed).map_err(|_| Error::Malformed("decompression failed"))?;
                    decompressed
                }
                _ => return Err(Error::Unsupported("unsupported ZIP compression method")),
            };

            if png_data.len() < 24 || !png_data.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err(Error::Malformed("corrupt PNG thumbnail"));
            }

            let width = u32::from_be_bytes(png_data.get(16..20).ok_or(Error::Malformed("width"))?.try_into().map_err(|_| Error::Malformed("width"))?);
            let height =
                u32::from_be_bytes(png_data.get(20..24).ok_or(Error::Malformed("height"))?.try_into().map_err(|_| Error::Malformed("height"))?);

            return Ok(Preview { data: png_data, width, height });
        }

        // Advance to next entry
        offset = data_end;
    }

    Err(Error::Unsupported("no embedded thumbnail in ZIP archive"))
}
