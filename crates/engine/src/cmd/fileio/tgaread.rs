//! Reading Targa files (`.tga`, `.vda`, `.icb`, `.vst`) for File › Open and File › Place, from the
//! Truevision TGA File Format Specification, version 2.0. The `image` crate decodes the pixels.
//! A Targa file starts with no signature, so [`is_targa`] checks its header's fields instead, and
//! [`fix_alpha`] gives the decoded image the transparency the file stores. The file is untrusted:
//! every read is checked.

/// Does `bytes` start with a Targa header? The fields must hold values the specification defines:
/// a color map type of 0 or 1; an image type of 1, 2, 3, 9, 10 or 11 (color-mapped, true-color or
/// black-and-white, raw or run-length encoded), with a color map for the color-mapped types; a
/// color map entry size of 15, 16, 24 or 32 bits when there is a map; a width and a height; a pixel
/// depth of 8, 15, 16, 24 or 32 bits; and bits 6 and 7 of the image descriptor clear.
pub(super) fn is_targa(bytes: &[u8]) -> bool {
    let [_, map_type, image_type, _, _, _, _, entry_bits, _, _, _, _, w0, w1, h0, h1, depth, descriptor, ..] = *bytes else {
        return false;
    };
    matches!(map_type, 0 | 1)
        && matches!(image_type, 1 | 2 | 3 | 9 | 10 | 11)
        && (map_type == 1 || !matches!(image_type, 1 | 9))
        && (map_type == 0 || matches!(entry_bits, 15 | 16 | 24 | 32))
        && u16::from_le_bytes([w0, w1]) > 0
        && u16::from_le_bytes([h0, h1]) > 0
        && matches!(depth, 8 | 15 | 16 | 24 | 32)
        && descriptor & 0xc0 == 0
}

/// Give `img`, decoded from Targa file `bytes`, the transparency the file stores. Bits 0 to 3 of
/// the image descriptor give the number of attribute bits per pixel, and the extension area's
/// Attributes Type, when the file has one, says what they hold:
/// - with no attribute bits, or attribute data the Attributes Type calls undefined (1 or 2), the
///   image is opaque, whatever a 32-bit pixel's fourth byte holds (the decoder reads it as alpha);
/// - with one bit, a 16-bit true-color pixel takes that bit as its alpha (clear: transparent), and
///   a color-mapped pixel takes the bit of the 16-bit color map entry it indexes; the decoder drops
///   the bit;
/// - with more, the decoder's alpha stays, and colors the Attributes Type calls premultiplied (4)
///   are divided by it.
///
/// An Attributes Type of 0 ("no alpha") with attribute bits in the descriptor counts as alpha.
pub(super) fn fix_alpha(bytes: &[u8], img: &mut image::RgbaImage) {
    let Some(bits) = bytes.get(17).map(|d| d & 0x0f) else { return };
    let kind = attributes_type(bytes);
    if bits == 0 || matches!(kind, Some(1 | 2)) {
        img.pixels_mut().for_each(|p| p.0[3] = 255);
        return;
    }
    if bits == 1 {
        one_bit_alpha(bytes, img);
    }
    if kind == Some(4) {
        for p in img.pixels_mut() {
            let a = u16::from(p.0[3]);
            for c in &mut p.0[..3] {
                // A fully transparent pixel keeps its colors.
                if let Some(v) = (u16::from(*c) * 255 + a / 2).checked_div(a) {
                    *c = v.min(255) as u8;
                }
            }
        }
    }
}

/// The Attributes Type of a Targa 2.0 file's extension area: `None` without the footer's
/// `TRUEVISION-XFILE.` signature, an extension area, or one of the 495 bytes the specification
/// gives it.
fn attributes_type(bytes: &[u8]) -> Option<u8> {
    let footer = bytes.len().checked_sub(26)?;
    if bytes.get(footer + 8..)? != b"TRUEVISION-XFILE.\0" {
        return None;
    }
    let at = usize::try_from(u32::from_le_bytes(bytes.get(footer..footer + 4)?.try_into().ok()?)).ok()?;
    // The area follows the header and ends before the footer; its first field is its size.
    if at < 18 || at.checked_add(495)? > footer || u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]) != 495 {
        return None;
    }
    bytes.get(at + 494).copied()
}

/// The attribute bit of each pixel of a Targa image with one attribute bit per pixel, as `img`'s
/// alpha: the top bit of a 16-bit true-color pixel, or of the 16-bit color map entry an 8- or
/// 16-bit index names, raw or run-length encoded. Rows and columns are stored in the order the
/// image descriptor's bits 5 (top to bottom) and 4 (right to left) give. Other images, and pixels
/// the data doesn't reach, keep the alpha they have.
fn one_bit_alpha(bytes: &[u8], img: &mut image::RgbaImage) {
    let [id_len, map_type, image_type, o0, o1, m0, m1, entry_bits, _, _, _, _, w0, w1, h0, h1, depth, descriptor, ..] = *bytes else {
        return;
    };
    let (w, h) = (u32::from(u16::from_le_bytes([w0, w1])), u32::from(u16::from_le_bytes([h0, h1])));
    let mapped = matches!(image_type, 1 | 9);
    // The number of bytes each pixel takes.
    let size = match image_type {
        2 | 10 if depth == 16 => 2,
        1 | 9 if map_type == 1 && entry_bits == 16 && matches!(depth, 8 | 16) => usize::from(depth / 8),
        _ => return,
    };
    if img.dimensions() != (w, h) {
        return;
    }
    let (first, entries) = (u16::from_le_bytes([o0, o1]), u16::from_le_bytes([m0, m1]));
    let map_at = 18 + usize::from(id_len);
    let map_len = if map_type == 1 { usize::from(entries) * usize::from(entry_bits).div_ceil(8) } else { 0 };
    // The attribute bit is the top bit of the little-endian pixel or color map entry.
    let attribute = |at: usize| -> Option<bool> {
        let px = bytes.get(at..at.checked_add(size)?)?;
        let high = if mapped {
            let index = match *px {
                [i] => u16::from(i),
                [lo, hi] => u16::from_le_bytes([lo, hi]),
                _ => return None,
            };
            let entry = index.checked_sub(first).filter(|e| *e < entries)?;
            *bytes.get(map_at + usize::from(entry) * 2 + 1)?
        } else {
            *px.get(1)?
        };
        Some(high & 0x80 != 0)
    };
    let (top_down, right_to_left) = (descriptor & 0x20 != 0, descriptor & 0x10 != 0);
    let total = u64::from(w) * u64::from(h);
    let mut set = |i: u64, opaque: bool| {
        let (row, col) = ((i / u64::from(w)) as u32, (i % u64::from(w)) as u32);
        let y = if top_down { row } else { h - 1 - row };
        let x = if right_to_left { w - 1 - col } else { col };
        img.get_pixel_mut(x, y).0[3] = if opaque { 255 } else { 0 };
    };
    let mut at = map_at + map_len;
    let mut i = 0;
    while i < total {
        if image_type < 9 {
            let Some(opaque) = attribute(at) else { return };
            set(i, opaque);
            (i, at) = (i + 1, at + size);
            continue;
        }
        let Some(&packet) = bytes.get(at) else { return };
        let count = u64::from(packet & 0x7f) + 1;
        at += 1;
        if packet & 0x80 != 0 {
            // A run packet repeats one pixel.
            let Some(opaque) = attribute(at) else { return };
            at += size;
            for _ in 0..count.min(total - i) {
                set(i, opaque);
                i += 1;
            }
        } else {
            for _ in 0..count.min(total - i) {
                let Some(opaque) = attribute(at) else { return };
                set(i, opaque);
                (i, at) = (i + 1, at + size);
            }
        }
    }
}
