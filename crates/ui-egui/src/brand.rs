//! The VectorCraft brand mark: the app icon (the engraved dragon, `assets/app-icon/`, see its
//! README). The PNG carries the icon's rounded corners; it is decoded once and shared by the app
//! bar's `Picture` and the About box, which upload and mipmap it themselves.

use std::sync::{Arc, OnceLock};

use egui::{Color32, ColorImage};

/// 128 px: sharp at 44 pt on a 2× display.
const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.vectorcraft.png");

/// The key a `Picture` stores the brand mark's texture under.
pub(crate) const BRAND_KEY: u64 = 0x5643_4252_414e_4421;

/// The icon's pixels (one transparent pixel if it couldn't be decoded, which a test rules out).
fn decode() -> ColorImage {
    match craft_image_codecs::decode_as(craft_image_codecs::Format::Png, ICON_PNG) {
        Ok(img) => ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], &img.to_rgba8()),
        Err(_) => ColorImage::filled([1, 1], Color32::TRANSPARENT),
    }
}

/// The brand mark's image, decoded on first use.
pub(crate) fn image() -> Arc<ColorImage> {
    static IMAGE: OnceLock<Arc<ColorImage>> = OnceLock::new();
    IMAGE.get_or_init(|| Arc::new(decode())).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A broken or replaced asset falls back to one transparent pixel, and the bar and About lose
    /// their mark.
    #[test]
    fn the_icon_is_not_the_transparent_fallback() {
        let img = image();
        let [w, h] = img.size;
        assert!(w > 1 && h > 1, "{:?}", img.size);
        let centre = img.pixels[(h / 2) * w + w / 2].a();
        assert_eq!(centre, 255, "the centre is opaque");
        assert!(img.pixels[0].a() < centre, "the rounded corner is more transparent than the centre");
    }
}
