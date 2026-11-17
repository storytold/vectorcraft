//! GIF export: a palette image ([`super::quantize`]) as a one-frame GIF89a with its transparent
//! entry, optionally interlaced (rows in four passes, so the image builds up while it loads).

use super::quantize::Indexed;

/// Encode `img` as a GIF file.
pub fn encode(img: &Indexed, interlaced: bool) -> Result<Vec<u8>, String> {
    if img.width > u32::from(u16::MAX) || img.height > u32::from(u16::MAX) {
        return Err(format!("{} × {} pixels is too large for GIF (at most 65535 pixels a side): lower the resolution", img.width, img.height));
    }
    if img.width == 0
        || img.height == 0
        || img.indices.len() != img.width as usize * img.height as usize
        || img.palette.is_empty()
        || img.palette.len() > 256
    {
        return Err("GIF encoding failed: the palette image is malformed".into());
    }
    craft_gif::write::encode_indexed(img.width, img.height, &img.palette, &img.indices, img.transparent, interlaced)
        .map_err(|e| format!("GIF encoding failed: {e}"))
}
