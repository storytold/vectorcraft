//! JPEG 2000 JP2 and codestream images for File › Open and File › Place. The decoded image is
//! bounded before allocation and converted to RGBA for the common PNG storage path.

use hayro_jpeg2000::{ColorSpace, DecodeSettings, DecoderContext, Image};
use vectorcraft_color::cms::{IccProfile, Intent};

/// Recognize a JP2 signature box or the start of a raw codestream. The decoder checks the rest.
pub(super) fn is_jpeg2000(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\0\0\0\x0cjP  \r\n\x87\n") || bytes.starts_with(b"\xff\x4f")
}

/// Boxes at one JP2 nesting level. Invalid lengths stop the walk at that level.
pub(super) fn boxes(b: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let header = b.get(at..at.checked_add(8)?)?;
        let size = u32::from_be_bytes(header.get(..4)?.try_into().ok()?);
        let (length, head) = match size {
            0 => (b.len().checked_sub(at)?, 8usize),
            1 => (usize::try_from(u64::from_be_bytes(b.get(at.checked_add(8)?..at.checked_add(16)?)?.try_into().ok()?)).ok()?, 16usize),
            n => (n as usize, 8usize),
        };
        if length < head {
            return None;
        }
        let end = at.checked_add(length)?;
        let data = b.get(at.checked_add(head)?..end)?;
        let ty = header.get(4..8)?;
        at = end;
        Some((ty, data))
    })
}

/// The decoder ignores JPX layers and composition and silently uses the last top-level jp2c.
fn unsupported_jpx_content(bytes: &[u8]) -> bool {
    fn association_has_content(data: &[u8], depth: usize) -> bool {
        if depth >= 16 {
            return true;
        }
        boxes(data).any(|(ty, child)| {
            matches!(ty, b"jp2c" | b"jpch" | b"jplh" | b"comp" | b"ftbl") || (ty == b"asoc" && association_has_content(child, depth + 1))
        })
    }
    let mut codestreams = 0u8;
    for (ty, data) in boxes(bytes) {
        match ty {
            b"jp2c" => {
                codestreams = codestreams.saturating_add(1);
                if codestreams > 1 {
                    return true;
                }
            }
            b"jpch" | b"jplh" | b"comp" | b"ftbl" => return true,
            b"asoc" if association_has_content(data, 0) => return true,
            _ => {}
        }
    }
    false
}

/// The largest embedded ICC profile read.
const MAX_PROFILE: usize = 4 * 1024 * 1024;
/// A codestream numbers its tiles 0 to 65,534 (Isot in SOT, ITU-T T.800 A.4.2).
const MAX_TILES: u64 = 65_535;
/// Bytes the decoder keeps for each component of each tile before decoding any of them.
const TILE_COMPONENT_BYTES: u64 = 256;

/// SIZ follows SOC in the codestream; Lsiz includes its two length bytes. Returns the component
/// count, the image size and the tile count.
fn codestream_size(bytes: &[u8]) -> Option<(u64, u32, u32, u64)> {
    let stream = if bytes.starts_with(b"\xff\x4f") { bytes } else { boxes(bytes).find_map(|(ty, data)| (ty == b"jp2c").then_some(data))? };
    if stream.get(..4)? != b"\xff\x4f\xff\x51" {
        return None;
    }
    let length = u16::from_be_bytes(stream.get(4..6)?.try_into().ok()?) as usize;
    let count = u16::from_be_bytes(stream.get(40..42)?.try_into().ok()?) as usize;
    if count == 0 || length != 38usize.checked_add(count.checked_mul(3)?)? || stream.get(4..4usize.checked_add(length)?)?.len() != length {
        return None;
    }
    let be32 = |range: std::ops::Range<usize>| stream.get(range).and_then(|v| v.try_into().ok()).map(u32::from_be_bytes);
    let width = be32(8..12)?.checked_sub(be32(16..20)?)?;
    let height = be32(12..16)?.checked_sub(be32(20..24)?)?;
    // Tiles across and down (T.800 B-5): the grid from the tile offset, in whole tiles.
    let tiles = |grid: u32, offset: u32, tile: u32| match (grid.checked_sub(offset), tile) {
        (Some(span), 1..) => Some(u64::from(span).div_ceil(u64::from(tile))),
        _ => None,
    };
    let across = tiles(be32(8..12)?, be32(32..36)?, be32(24..28)?)?;
    let down = tiles(be32(12..16)?, be32(36..40)?, be32(28..32)?)?;
    Some((count as u64, width, height, across.checked_mul(down)?))
}

/// One `jp2h` header with at most one palette, component mapping and channel definition box, so
/// the checks below read the boxes the decoder uses.
fn single_header_boxes(bytes: &[u8]) -> bool {
    let mut headers = boxes(bytes).filter(|(ty, _)| *ty == b"jp2h");
    let (Some((_, header)), None) = (headers.next(), headers.next()) else {
        return false;
    };
    [b"pclr", b"cmap", b"cdef"].iter().all(|kind| boxes(header).filter(|(ty, _)| ty == kind).count() <= 1)
}

/// The number of channel definitions in a JP2 `cdef` box, `None` without one. Only color channels
/// 1 to n, in any order, followed by at most one opacity channel for the whole image are read;
/// any other layout (premultiplied opacity, opacity before a color, an unassociated channel) gives
/// `Some(0)`, which no image matches.
fn channel_definitions(bytes: &[u8]) -> Option<u64> {
    if bytes.starts_with(b"\xff\x4f") {
        return None;
    }
    let header = boxes(bytes).find_map(|(ty, data)| (ty == b"jp2h").then_some(data))?;
    let cdef = boxes(header).find_map(|(ty, data)| (ty == b"cdef").then_some(data))?;
    let count = cdef.get(..2).and_then(|n| n.try_into().ok()).map_or(0, |n: [u8; 2]| usize::from(u16::from_be_bytes(n)));
    let Some(entries) = cdef.get(2..).filter(|rest| count > 0 && rest.len() == count * 6) else {
        return Some(0);
    };
    // (channel, type, association) by channel.
    let mut defs: Vec<[u16; 3]> =
        entries.as_chunks::<6>().0.iter().map(|&[a, b, c, d, e, f]| [[a, b], [c, d], [e, f]].map(u16::from_be_bytes)).collect();
    defs.sort_unstable();
    let opacity = defs.last().is_some_and(|&[_, kind, association]| kind == 1 && association == 0);
    let colors = defs.len() - usize::from(opacity);
    let mut seen = vec![false; colors];
    let layout_ok = defs.iter().enumerate().all(|(i, &[channel, kind, association])| {
        usize::from(channel) == i
            && (i >= colors
                || (kind == 0
                    && usize::from(association).checked_sub(1).and_then(|slot| seen.get_mut(slot)).is_some_and(|s| !std::mem::replace(s, true))))
    });
    Some(if layout_ok { defs.len() as u64 } else { 0 })
}

/// The column count of a JP2 palette (`pclr`), `Ok(None)` without one. A component mapping (`cmap`)
/// beside it needs one entry per column, each naming a codestream component and, when it maps
/// through the palette, a palette column; anything else is `Err`.
fn palette_columns(bytes: &[u8], components: u64) -> Result<Option<u64>, ()> {
    if bytes.starts_with(b"\xff\x4f") {
        return Ok(None);
    }
    let Some(header) = boxes(bytes).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)) else {
        return Ok(None);
    };
    let Some(palette) = boxes(header).find_map(|(ty, data)| (ty == b"pclr").then_some(data)) else {
        return Ok(None);
    };
    let columns = palette.get(2).copied().ok_or(())?;
    // Each column's descriptor: unsigned, 1 to 31 bits, as codestream samples are.
    let depths = palette.get(3..3 + usize::from(columns)).ok_or(())?;
    if columns == 0 || depths.iter().any(|&d| d & 0x80 != 0 || (d & 0x7f) + 1 > 31) {
        return Err(());
    }
    // Without a mapping the decoder maps every column from the first component.
    let Some(mapping) = boxes(header).find_map(|(ty, data)| (ty == b"cmap").then_some(data)) else {
        return Ok(Some(u64::from(columns)));
    };
    if mapping.len() != usize::from(columns) * 4 {
        return Err(());
    }
    for &[c0, c1, kind, column] in mapping.as_chunks::<4>().0 {
        let valid = u64::from(u16::from_be_bytes([c0, c1])) < components
            && match kind {
                0 => true,
                1 => column < columns,
                _ => false,
            };
        if !valid {
            return Err(());
        }
    }
    Ok(Some(u64::from(columns)))
}

/// Decode within the raster allocation allowance, counting source samples, RGBA output, and
/// decoder intermediates. The decoder may need multiple full-sized component planes.
pub(super) fn decode(bytes: &[u8], max_alloc: u64) -> Result<image::RgbaImage, String> {
    if bytes.starts_with(b"\0\0\0\x0cjP  \r\n\x87\n") && unsupported_jpx_content(bytes) {
        return Err("the JPEG 2000 file has an unsupported color space or JPX content".into());
    }
    if bytes.starts_with(b"\0\0\0\x0cjP  \r\n\x87\n") && !single_header_boxes(bytes) {
        return Err("the JPEG 2000 file can't be read".into());
    }
    let (codestream_channels, declared_w, declared_h, tile_count) = codestream_size(bytes).ok_or("the JPEG 2000 file can't be read")?;
    if tile_count == 0 || tile_count > MAX_TILES {
        return Err("the JPEG 2000 file can't be read".into());
    }
    let declared_pixels = u64::from(declared_w).checked_mul(u64::from(declared_h));
    if declared_w == 0
        || declared_h == 0
        || declared_pixels.and_then(|n| n.checked_mul(codestream_channels)).and_then(|n| n.checked_mul(4)).is_none_or(|n| n > max_alloc)
        || tile_count.checked_mul(codestream_channels).and_then(|n| n.checked_mul(TILE_COMPONENT_BYTES)).is_none_or(|n| n > max_alloc)
    {
        return Err(format!("an image of {declared_w} × {declared_h} pixels is too large to draw"));
    }
    // The decoder copies an embedded profile (after a 3-byte `colr` header) when it reads the file.
    let header = boxes(bytes).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)).unwrap_or_default();
    if boxes(header).any(|(ty, data)| ty == b"colr" && data.len() > MAX_PROFILE + 3) {
        return Err("the JPEG 2000 file has an unsupported color space or JPX content".into());
    }
    let palette = palette_columns(bytes, codestream_channels).map_err(|()| "the JPEG 2000 file can't be read")?;
    // The decoder keeps each palette value in 8 bytes and each row in its own list.
    let palette_rows = boxes(header).find_map(|(ty, data)| (ty == b"pclr").then(|| data.get(..2)).flatten());
    if let (Some(columns), Some(&[r0, r1])) = (palette, palette_rows)
        && u64::from(u16::from_be_bytes([r0, r1])) * (columns * 8 + 24) > max_alloc
    {
        return Err(format!("an image of {declared_w} × {declared_h} pixels is too large to draw"));
    }
    let image = Image::new(bytes, &DecodeSettings::default()).map_err(|_| "the JPEG 2000 file can't be read")?;
    let (w, h) = (image.width(), image.height());
    let color = image.color_space();
    let channels = match color {
        ColorSpace::Gray => 1,
        ColorSpace::RGB => 3,
        ColorSpace::CMYK => 4,
        ColorSpace::Icc { num_channels: 1 | 3 | 4, profile } if profile.len() <= MAX_PROFILE => color.num_channels(),
        _ => return Err("the JPEG 2000 file has an unsupported color space or JPX content".into()),
    };
    let stride = u64::from(channels) + u64::from(image.has_alpha());
    // The decoder sizes its output by the palette's columns and writes one channel per mapping
    // entry, so the columns, the entries and the output channels must all match.
    if palette.is_some_and(|columns| columns != stride) {
        return Err("the JPEG 2000 file can't be read".into());
    }
    // The decoder reorders the channels by their definitions (`cdef`) and drops any channel without
    // one, so a definition is needed for every channel it writes.
    if channel_definitions(bytes).is_some_and(|count| count != stride || count != palette.unwrap_or(codestream_channels)) {
        return Err("the JPEG 2000 file can't be read".into());
    }
    let plane_channels = codestream_channels.max(stride).max(palette.unwrap_or(0));
    let too_large = || format!("an image of {w} × {h} pixels is too large to draw");
    let pixels = u64::from(w).checked_mul(u64::from(h)).ok_or_else(too_large)?;
    let size = pixels.checked_mul(stride).ok_or_else(too_large)?;
    let rgba_size = pixels.checked_mul(4).ok_or_else(too_large)?;
    // hayro 0.3.5 keeps a f32 plane per component, including alpha, and a second
    // f32 coefficient plane per component. Palette mapping or channel definitions
    // can temporarily clone the component planes. Reserve another 8 bytes per
    // pixel for IDWT and code-block scratch while samples and RGBA coexist.
    let plane_bytes = pixels.checked_mul(plane_channels).and_then(|n| n.checked_mul(4)).ok_or_else(too_large)?;
    let scratch_bytes = plane_bytes.checked_mul(2).and_then(|n| n.checked_add(pixels.checked_mul(8)?)).ok_or_else(too_large)?;
    let working =
        size.checked_add(rgba_size).and_then(|n| n.checked_add(plane_bytes)).and_then(|n| n.checked_add(scratch_bytes)).ok_or_else(too_large)?;
    if w == 0 || h == 0 || working > max_alloc {
        return Err(too_large());
    }
    let len = usize::try_from(size).map_err(|_| too_large())?;
    let mut samples = vec![0; len];
    image.decode_into(&mut samples, &mut DecoderContext::default()).map_err(|_| "the JPEG 2000 file can't be read")?;
    let profile = match color {
        ColorSpace::Icc { profile, .. } => {
            Some(IccProfile::from_bytes(None, profile).map_err(|_| "the JPEG 2000 file has an unsupported color space or JPX content")?)
        }
        _ => None,
    };
    let cms = vectorcraft_color::cms::active();
    let mut rgba = Vec::with_capacity(usize::try_from(rgba_size).map_err(|_| too_large())?);
    for sample in samples.chunks_exact(stride as usize) {
        let at = |i: usize| sample.get(i).copied().unwrap_or(0);
        let rgb = match (color, profile.as_ref()) {
            (ColorSpace::Gray, _) => [at(0); 3],
            (ColorSpace::RGB, _) => [at(0), at(1), at(2)],
            (ColorSpace::CMYK, _) => {
                cms.cmyk_to_srgb([at(0), at(1), at(2), at(3)].map(|v| f32::from(v) / 255.0), false).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            }
            (ColorSpace::Icc { .. }, Some(profile)) => {
                let mut values = [0.0f32; 4];
                for (dst, src) in values.iter_mut().zip(sample.iter().take(usize::from(channels))) {
                    *dst = f32::from(*src) / 255.0;
                }
                profile
                    .to_srgb_into(&values[..usize::from(channels)], Intent::RelativeColorimetric)
                    .ok_or("the JPEG 2000 file has an unsupported color space or JPX content")?
                    .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            }
            _ => return Err("the JPEG 2000 file has an unsupported color space or JPX content".into()),
        };
        let [r, g, b] = rgb;
        rgba.extend_from_slice(&[r, g, b, if image.has_alpha() { at(usize::from(channels)) } else { 255 }]);
    }
    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| "the JPEG 2000 file can't be read".into())
}
