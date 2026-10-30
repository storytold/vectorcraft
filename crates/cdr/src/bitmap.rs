//! Native CDR `bmp ` RI resources, decoded into shared PNG image blobs.
//!
//! Layout references: the MIT Kaitai CDR grammar (`bmp_chunk_data`) and the
//! MIT/Apache-2.0 TraceDraw format notes (`bmp` and the separate RI alpha mask).
//! See the crate README for provenance. The main resource's pixel header is
//! authoritative; a class-5 object's duplicate pixel metadata can disagree.
//! Native previews are never read by this module.

use std::io::{self, Write};

use image::ImageEncoder;
use vectorcraft_doc::ImageBlob;

/// A decoded native bitmap resource, ready for one or more image placements.
#[derive(Clone, Debug)]
pub(crate) struct Bitmap {
    /// The native image identifier referenced by class-5 coordinate records.
    pub id: u32,
    /// The resource's pixel width, independent of its placement size.
    pub width: u32,
    /// The resource's pixel height, independent of its placement size.
    pub height: u32,
    /// A content-derived document image key, shared by identical decoded PNGs.
    pub key: String,
    /// Straight RGBA PNG bytes of the embedded native artwork.
    pub blob: ImageBlob,
    /// True when CMYK ink samples were converted without the source ICC profile.
    pub cmyk_converted: bool,
}

/// Cumulative decoding allowances for one import, shared by all image resources.
/// Pixel allowance is charged before allocation; failed or distinct equivalent
/// decodes do not refund it. Identical raw resource aliases are skipped by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BitmapBudget {
    remaining_pixels: u64,
    remaining_encoded_bytes: usize,
}

impl Default for BitmapBudget {
    /// Allow at most 32 million decoded pixels and 128 MiB of generated PNG bytes.
    fn default() -> Self {
        Self { remaining_pixels: 32_000_000, remaining_encoded_bytes: 128 * 1024 * 1024 }
    }
}

/// A validated RI block. All offsets are relative to the enclosing bmp payload.
struct Ri {
    header_end: usize,
    end: usize,
    model: u32,
    width: u32,
    height: u32,
    bpp: u32,
    stride: usize,
    pixel_bytes: usize,
}

/// Read one little-endian u16 with checked offset arithmetic and input bounds.
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset.checked_add(2).ok_or("CDR bitmap offset overflow")?;
    Ok(u16::from_le_bytes(bytes.get(offset..end).ok_or("truncated CDR bitmap integer")?.try_into().map_err(|_| "truncated CDR bitmap integer")?))
}

/// Read one little-endian u32 with checked offset arithmetic and input bounds.
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset.checked_add(4).ok_or("CDR bitmap offset overflow")?;
    Ok(u32::from_le_bytes(bytes.get(offset..end).ok_or("truncated CDR bitmap integer")?.try_into().map_err(|_| "truncated CDR bitmap integer")?))
}

/// Add a fixed field displacement without overflowing wasm32's usize.
fn add(start: usize, amount: usize) -> Result<usize, String> {
    start.checked_add(amount).ok_or_else(|| "CDR bitmap offset overflow".into())
}

/// Parse the RI signature, block/header bounds and its dimensional fields.
/// The 78-byte header is the modern native layout, not BITMAPINFOHEADER.
fn ri_header(bytes: &[u8], start: usize) -> Result<Ri, String> {
    if bytes.get(start..add(start, 2)?) != Some(b"RI") {
        return Err("CDR bitmap RI signature is missing".into());
    }
    let len = u32_at(bytes, add(start, 2)?)? as usize;
    let header = u32_at(bytes, add(start, 10)?)? as usize;
    if header < 78 || len < header {
        return Err("invalid CDR bitmap RI header length".into());
    }
    let end = add(start, len)?;
    let header_end = add(start, header)?;
    if end > bytes.len() || header_end > end {
        return Err("CDR bitmap RI block exceeds its resource".into());
    }
    if u32_at(bytes, add(start, 30)?)? != 1 {
        return Err("invalid CDR bitmap plane count".into());
    }
    Ok(Ri {
        header_end,
        end,
        model: u32_at(bytes, add(start, 14)?)?,
        width: u32_at(bytes, add(start, 22)?)?,
        height: u32_at(bytes, add(start, 26)?)?,
        bpp: u32_at(bytes, add(start, 34)?)?,
        stride: u32_at(bytes, add(start, 38)?)? as usize,
        pixel_bytes: u32_at(bytes, add(start, 42)?)? as usize,
    })
}

/// Calculate four-byte row alignment and validate the stored pixel-byte count.
fn row_layout(header: &Ri) -> Result<(usize, usize), String> {
    if header.width == 0 || header.height == 0 || header.width > 32_768 || header.height > 32_768 {
        return Err("CDR bitmap dimensions are empty or exceed 32768 pixels".into());
    }
    let bits = (header.width as usize).checked_mul(header.bpp as usize).ok_or("CDR bitmap row size overflow")?;
    let stride = bits.checked_add(31).and_then(|n| (n / 32).checked_mul(4)).ok_or("CDR bitmap row size overflow")?;
    let required = stride.checked_mul(header.height as usize).ok_or("CDR bitmap pixel size overflow")?;
    if header.stride != stride || header.pixel_bytes != required {
        return Err("CDR bitmap row stride or pixel-byte count is inconsistent".into());
    }
    Ok((stride, required))
}

/// Borrow raw rows only after checking that they lie inside their RI block.
fn rows<'a>(bytes: &'a [u8], header: &Ri, start: usize) -> Result<&'a [u8], String> {
    let end = add(start, header.pixel_bytes)?;
    if end > header.end {
        return Err("CDR bitmap pixel rows exceed their RI block".into());
    }
    bytes.get(start..end).ok_or_else(|| "truncated CDR bitmap pixel rows".into())
}

/// A PNG sink that cannot grow beyond its encoded-byte allowance and reports
/// allocation failure instead of requesting an unchecked Vec reallocation.
struct PngOutput {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for PngOutput {
    /// Append an encoder chunk only when both quota and allocation permit it.
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let available = self.limit.saturating_sub(self.bytes.len());
        if input.len() > available {
            return Err(io::Error::other("CDR encoded bitmap quota exceeded"));
        }
        self.bytes.try_reserve(input.len()).map_err(|_| io::Error::other("CDR PNG allocation failed"))?;
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }

    /// The sink has no buffered external destination to flush.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Convert a CMYK ink byte and black ink byte to one profile-free RGB channel.
/// This is round((1-ink/255)*(1-black/255)*255); it does not use ICC colour data.
fn ink_channel(ink: u8, black: u8) -> u8 {
    (((255 - u32::from(ink)) * (255 - u32::from(black)) + 127) / 255) as u8
}

/// Decode a native bmp payload, returning None for unsupported RI/pixel modes.
/// Supported raw pixels are model3/32-bit CMYK, model1/24-bit BGR, model1/8-bit
/// and model1/1-bit BGR-palette indices, and model5/8-bit grayscale. The supplied
/// user drawing verifies CMYK and separate alpha; the other modes have synthetic
/// coverage from the public descriptions. A nonzero separate mask is validated
/// and retained at its full 8-bit precision. Malformed bounds or exhausted
/// cumulative allowances return an error. No preview pixels are ever substituted.
pub(crate) fn decode(bytes: &[u8], budget: &mut BitmapBudget) -> Result<Option<Bitmap>, String> {
    let id = u32_at(bytes, 0)?;
    // Format 7+ puts RI at byte40. Format6's documented gap is four bytes shorter.
    let start = if bytes.get(40..42) == Some(b"RI") {
        40
    } else if bytes.get(36..38) == Some(b"RI") {
        36
    } else {
        return Ok(None);
    };
    let header = ri_header(bytes, start)?;
    let cmyk = header.model == 3 && header.bpp == 32;
    let bgr = header.model == 1 && header.bpp == 24;
    let gray = header.model == 5 && header.bpp == 8;
    let indexed = header.model == 1 && matches!(header.bpp, 1 | 8);
    if !(cmyk || bgr || gray || indexed) {
        return Ok(None);
    }
    let (stride, _) = row_layout(&header)?;
    let mut pixel_start = header.header_end;
    let mut palette = Vec::<[u8; 3]>::new();
    if indexed {
        let count = u16_at(bytes, add(pixel_start, 2)?)? as usize;
        let max = if header.bpp == 1 { 2 } else { 256 };
        if count == 0 || count > max {
            return Err("invalid CDR bitmap palette size".into());
        }
        pixel_start = add(pixel_start, 4)?;
        let len = count.checked_mul(3).ok_or("CDR bitmap palette size overflow")?;
        let end = add(pixel_start, len)?;
        if end > header.end {
            return Err("CDR bitmap palette exceeds its RI block".into());
        }
        let colors = bytes.get(pixel_start..end).ok_or("truncated CDR bitmap palette")?;
        palette.try_reserve_exact(count).map_err(|_| "CDR palette allocation failed")?;
        for [b, g, r] in colors.as_chunks::<3>().0 {
            palette.push([*r, *g, *b]);
        }
        pixel_start = end;
    }
    let color_rows = rows(bytes, &header, pixel_start)?;
    let color_end = add(pixel_start, header.pixel_bytes)?;
    let mask_offset = u32_at(bytes, 24)? as usize;
    let mut alpha = None;
    if mask_offset != 0 {
        let mask_start = add(mask_offset, 8)?;
        if mask_start < color_end || mask_start < header.end {
            return Err("CDR bitmap alpha block overlaps colour pixels".into());
        }
        let mask = ri_header(bytes, mask_start)?;
        if mask.model != 99 || mask.bpp != 8 {
            return Ok(None);
        }
        if mask.width != header.width || mask.height != header.height {
            return Err("CDR bitmap alpha dimensions disagree with colour pixels".into());
        }
        let (mask_stride, _) = row_layout(&mask)?;
        alpha = Some((rows(bytes, &mask, mask.header_end)?, mask_stride));
    }
    let pixels = u64::from(header.width) * u64::from(header.height);
    budget.remaining_pixels = budget.remaining_pixels.checked_sub(pixels).ok_or("CDR decoded bitmap pixel quota exceeded")?;
    let rgba_len = usize::try_from(pixels).ok().and_then(|n| n.checked_mul(4)).ok_or("CDR decoded bitmap size overflow")?;
    let mut rgba = Vec::<u8>::new();
    rgba.try_reserve_exact(rgba_len).map_err(|_| "CDR decoded bitmap allocation failed")?;
    for y in 0..header.height as usize {
        // Both native colour rows and the separate mask are bottom-up.
        let row = header.height as usize - 1 - y;
        let at = row.checked_mul(stride).ok_or("CDR bitmap row offset overflow")?;
        let source = color_rows.get(at..add(at, stride)?).ok_or("truncated CDR bitmap row")?;
        let mask = if let Some((rows, mask_stride)) = alpha {
            let at = row.checked_mul(mask_stride).ok_or("CDR alpha row offset overflow")?;
            Some(rows.get(at..add(at, header.width as usize)?).ok_or("truncated CDR alpha row")?)
        } else {
            None
        };
        for x in 0..header.width as usize {
            let rgb = if cmyk {
                let at = x.checked_mul(4).ok_or("CDR CMYK pixel offset overflow")?;
                let px = source.get(at..add(at, 4)?).ok_or("truncated CDR CMYK pixel")?;
                let [c, m, y, k]: [u8; 4] = px.try_into().map_err(|_| "truncated CDR CMYK pixel")?;
                [ink_channel(c, k), ink_channel(m, k), ink_channel(y, k)]
            } else if bgr {
                let at = x.checked_mul(3).ok_or("CDR BGR pixel offset overflow")?;
                let px = source.get(at..add(at, 3)?).ok_or("truncated CDR BGR pixel")?;
                let [b, g, r]: [u8; 3] = px.try_into().map_err(|_| "truncated CDR BGR pixel")?;
                [r, g, b]
            } else if indexed {
                let index = if header.bpp == 1 {
                    (source.get(x / 8).copied().ok_or("truncated CDR monochrome pixel")? >> (7 - x % 8)) & 1
                } else {
                    source.get(x).copied().ok_or("truncated CDR palette index")?
                };
                palette.get(index as usize).copied().ok_or("CDR bitmap index exceeds its palette")?
            } else {
                let gray = source.get(x).copied().ok_or("truncated CDR grayscale pixel")?;
                [gray; 3]
            };
            let alpha = match mask {
                Some(mask) => mask.get(x).copied().ok_or("truncated CDR bitmap alpha pixel")?,
                None => 255,
            };
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
        }
    }
    let mut output = PngOutput { bytes: Vec::new(), limit: budget.remaining_encoded_bytes };
    image::codecs::png::PngEncoder::new(&mut output)
        .write_image(&rgba, header.width, header.height, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("CDR embedded bitmap PNG encoding failed: {e}"))?;
    budget.remaining_encoded_bytes = budget.remaining_encoded_bytes.checked_sub(output.bytes.len()).ok_or("CDR encoded bitmap quota exceeded")?;
    let blob = ImageBlob::png(output.bytes);
    let key = blob.content_key();
    Ok(Some(Bitmap { id, width: header.width, height: header.height, key, blob, cmyk_converted: cmyk }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    /// Write a little-endian field in a synthetic, test-owned header.
    fn field(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Make an original RI block with explicit padded raw rows.
    fn ri(model: u32, width: u32, height: u32, bpp: u32, palette: &[u8], pixels: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 78];
        bytes[..2].copy_from_slice(b"RI");
        field(&mut bytes, 2, (78 + palette.len() + pixels.len()) as u32);
        field(&mut bytes, 10, 78);
        field(&mut bytes, 14, model);
        field(&mut bytes, 18, 1);
        field(&mut bytes, 22, width);
        field(&mut bytes, 26, height);
        field(&mut bytes, 30, 1);
        field(&mut bytes, 34, bpp);
        field(&mut bytes, 38, (width * bpp).div_ceil(32) * 4);
        field(&mut bytes, 42, pixels.len() as u32);
        bytes.extend_from_slice(palette);
        bytes.extend_from_slice(pixels);
        bytes
    }

    /// Wrap an original RI colour block and optional RI alpha in a bmp payload.
    fn resource(color: Vec<u8>, mask: Option<Vec<u8>>) -> Vec<u8> {
        let mut bytes = vec![0; 40];
        field(&mut bytes, 0, 42);
        bytes.extend(color);
        if let Some(mask) = mask {
            let at = bytes.len() as u32 - 8;
            field(&mut bytes, 24, at);
            bytes.extend(mask);
        }
        bytes
    }

    /// Decode a test PNG back into straight RGBA for independent pixel assertions.
    fn rgba(bitmap: &Bitmap) -> Vec<u8> {
        image::load_from_memory(&bitmap.blob.bytes).unwrap().to_rgba8().into_raw()
    }

    /// CMYK inks and alpha remain separate; rows are flipped exactly once.
    #[test]
    fn cmyk_and_continuous_alpha_keep_top_down_pixels() {
        let color = ri(3, 2, 2, 32, &[], &[0, 0, 255, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 255, 0, 0]);
        let mask = ri(99, 2, 2, 8, &[], &[128, 0, 0, 0, 255, 64, 0, 0]);
        let b = decode(&resource(color, Some(mask)), &mut BitmapBudget::default()).unwrap().unwrap();
        assert_eq!((b.id, b.width, b.height), (42, 2, 2));
        assert!(b.cmyk_converted);
        assert_eq!(rgba(&b), [0, 255, 255, 255, 255, 0, 255, 64, 255, 255, 0, 128, 0, 0, 0, 0]);
    }

    /// The three-byte BGR rows use four-byte alignment without reading padding.
    #[test]
    fn bgr_rows_honor_padding_and_orientation() {
        let b = resource(ri(1, 1, 2, 24, &[], &[255, 0, 0, 99, 0, 0, 255, 77]), None);
        let b = decode(&b, &mut BitmapBudget::default()).unwrap().unwrap();
        assert_eq!(rgba(&b), [255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(!b.cmyk_converted);
    }

    /// Palette indices are not grayscale and monochrome bits are most-significant first.
    #[test]
    fn palette_indices_and_monochrome_bits_select_bgr_colors() {
        let palette = [0, 0, 2, 0, 0, 0, 255, 0, 255, 0];
        let b = resource(ri(1, 2, 1, 1, &palette, &[0b0100_0000, 0, 0, 0]), None);
        let b = decode(&b, &mut BitmapBudget::default()).unwrap().unwrap();
        assert_eq!(rgba(&b), [255, 0, 0, 255, 0, 255, 0, 255]);
        let b = resource(ri(1, 2, 1, 8, &palette, &[1, 0, 0, 0]), None);
        let b = decode(&b, &mut BitmapBudget::default()).unwrap().unwrap();
        assert_eq!(rgba(&b), [0, 255, 0, 255, 255, 0, 0, 255]);
    }

    /// Model5 grayscale stores luminance samples directly, with no palette.
    #[test]
    fn grayscale_uses_sample_values() {
        let b = resource(ri(5, 2, 1, 8, &[], &[32, 192, 0, 0]), None);
        let b = decode(&b, &mut BitmapBudget::default()).unwrap().unwrap();
        assert_eq!(rgba(&b), [32, 32, 32, 255, 192, 192, 192, 255]);
    }

    /// Unknown pixel models decline conversion; malformed known records fail.
    #[test]
    fn unsupported_modes_and_invalid_bounds_are_distinct() {
        let b = resource(ri(1234, 1, 1, 8, &[], &[0; 4]), None);
        assert!(decode(&b, &mut BitmapBudget::default()).unwrap().is_none());
        let mut b = resource(ri(3, 1, 1, 32, &[], &[0; 4]), None);
        field(&mut b, 78, 4000);
        assert!(decode(&b, &mut BitmapBudget::default()).is_err());
        let mut b = resource(ri(3, 1, 1, 32, &[], &[0; 4]), None);
        field(&mut b, 24, 32);
        assert!(decode(&b, &mut BitmapBudget::default()).is_err());
    }

    /// Both pixel and encoded-output budgets accumulate across successful resources.
    #[test]
    fn cumulative_budgets_prevent_oversized_decode_or_encoding() {
        let b = resource(ri(3, 1, 1, 32, &[], &[0; 4]), None);
        let mut budget = BitmapBudget { remaining_pixels: 1, remaining_encoded_bytes: 1024 };
        assert!(decode(&b, &mut budget).unwrap().is_some());
        assert!(decode(&b, &mut budget).unwrap_err().contains("pixel quota"));
        let mut budget = BitmapBudget { remaining_pixels: 1, remaining_encoded_bytes: 1 };
        assert!(decode(&b, &mut budget).unwrap_err().contains("PNG encoding"));
    }

    /// Every prefix that cuts the required RI block is rejected without indexing panics.
    #[test]
    fn truncated_prefixes_never_decode_partial_pixels() {
        let b = resource(ri(3, 1, 1, 32, &[], &[0; 4]), None);
        for n in 0..b.len() {
            assert!(!matches!(decode(&b[..n], &mut BitmapBudget::default()), Ok(Some(_))));
        }
    }
}
