//! Decompression for CorelDRAW CPng chunk streams.

use crate::error::Error;
use flate2::read::ZlibDecoder;
use std::io::Read;

/// The header of a CorelDRAW CPng compressed stream.
#[derive(Debug, Clone, Copy)]
pub struct CPngHeader {
    pub uncompressed_size: u32,
    pub compressed_size: u32,
}

/// Parse and decompress all CPng streams in `bytes`.
pub fn decompress_cpng(bytes: &[u8], max_decompressed: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut found_any = false;

    while pos + 8 <= bytes.len() {
        let Some(rem) = bytes.get(pos..) else { break };
        let Some(rel_pos) = rem.windows(4).position(|w| w == b"CPng") else { break };
        let cpng_pos = pos + rel_pos;
        let payload_offset = cpng_pos.saturating_add(8);
        if payload_offset >= bytes.len() {
            break;
        }

        let Some(payload) = bytes.get(payload_offset..) else { break };
        let remaining_budget = max_decompressed.saturating_sub(out.len());
        if remaining_budget == 0 {
            break;
        }

        let mut decoder = ZlibDecoder::new(payload);
        let mut chunk_out = Vec::new();
        let mut take = (&mut decoder).take(remaining_budget as u64 + 1);
        if take.read_to_end(&mut chunk_out).is_ok() && !chunk_out.is_empty() {
            if out.len() + chunk_out.len() > max_decompressed {
                return Err(Error::Limit("decompressed stream exceeded limit"));
            }
            out.extend_from_slice(&chunk_out);
            found_any = true;
        }

        pos = cpng_pos + 4;
    }

    if !found_any && out.is_empty() {
        return Err(Error::Malformed("missing or invalid CPng compressed streams"));
    }

    Ok(out)
}

/// Decompress a CorelDRAW `LIST [cmpr]` payload (the slice after the `"cmpr"` tag)
/// into `(decompressed_riff_stream, block_sizes_table)`.
pub fn decompress_cmpr(cmpr_inner: &[u8], max_decompressed: usize) -> Result<(Vec<u8>, Vec<u32>), Error> {
    if cmpr_inner.len() < 24 {
        return Err(Error::Malformed("cmpr chunk header too short"));
    }
    let csize0 = u32::from_le_bytes([cmpr_inner[0], cmpr_inner[1], cmpr_inner[2], cmpr_inner[3]]) as usize;
    let usize0 = u32::from_le_bytes([cmpr_inner[4], cmpr_inner[5], cmpr_inner[6], cmpr_inner[7]]) as usize;
    let csize1 = u32::from_le_bytes([cmpr_inner[8], cmpr_inner[9], cmpr_inner[10], cmpr_inner[11]]) as usize;
    let usize1 = u32::from_le_bytes([cmpr_inner[12], cmpr_inner[13], cmpr_inner[14], cmpr_inner[15]]) as usize;

    if usize0 == 0 || csize0 < 8 {
        return Err(Error::Malformed("empty cmpr stream"));
    }
    if usize0.saturating_add(usize1) > max_decompressed {
        return Err(Error::Limit("cmpr uncompressed size exceeds limit"));
    }

    let cpng0_end = 16usize.saturating_add(csize0).min(cmpr_inner.len());
    let Some(cpng0) = cmpr_inner.get(16..cpng0_end) else {
        return Err(Error::Malformed("truncated CPng[0] stream"));
    };
    if !cpng0.starts_with(b"CPng") || cpng0.len() < 8 {
        return Err(Error::Malformed("missing CPng[0] signature"));
    }
    let stream = zlib_decompress(&cpng0[8..], max_decompressed)?;
    if stream.is_empty() {
        return Err(Error::Malformed("empty decompressed CPng[0] stream"));
    }

    let mut block_sizes = Vec::new();
    let rem = cmpr_inner.get(cpng0_end..).unwrap_or_default();
    if let Some(rel) = rem.windows(4).position(|w| w == b"CPng") {
        let cpng1_slice = rem.get(rel..rel.saturating_add(csize1).min(rem.len())).unwrap_or_default();
        if let Some(zlib_bytes) = cpng1_slice.get(8..)
            && let Ok(raw_bs) = zlib_decompress(zlib_bytes, max_decompressed.saturating_sub(stream.len()))
        {
            block_sizes.reserve(raw_bs.len() / 4);
            for &chunk in raw_bs.as_chunks::<4>().0 {
                block_sizes.push(u32::from_le_bytes(chunk));
            }
        }
    }

    Ok((stream, block_sizes))
}

fn zlib_decompress(payload: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let mut decoder = ZlibDecoder::new(payload);
    let mut out = Vec::new();
    let mut take = (&mut decoder).take(limit as u64 + 1);
    take.read_to_end(&mut out).map_err(|_| Error::Malformed("invalid zlib stream in CPng"))?;
    if out.len() > limit {
        return Err(Error::Limit("decompressed stream exceeded limit"));
    }
    Ok(out)
}
