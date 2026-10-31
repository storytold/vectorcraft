//! macOS: Sample Color from Screen (`vectorcraft_ui_egui::screen_color`) through the system's color
//! sampler, `NSColorSampler` (macOS 10.15 and later), which "displays the system’s color-sampling
//! interface and returns the selected color to your app"
//! (<https://developer.apple.com/documentation/appkit/nscolorsampler>). AppKit calls the handler on
//! the main thread with the color the person clicked, or with nil when they pressed Escape. The
//! color is converted to sRGB before it is sent to the UI; AppKit clips a color outside sRGB to it
//! (Display P3 red arrives as sRGB red).
//!
//! `showSamplerWithSelectionHandler:` has no safe binding; this module allows `unsafe_code`.
#![allow(unsafe_code)]

use std::sync::mpsc::channel;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSColor, NSColorSampler, NSColorSpace};
use vectorcraft_ui_egui::screen_color::{Answer, StartSampler};

/// The system's color sampler; `ui` is woken when it answers.
pub fn sampler(ui: egui::Context) -> StartSampler {
    Box::new(move || {
        MainThreadMarker::new().ok_or("the color sampler can only be shown from the main thread")?;
        let (tx, rx) = channel();
        let ui = ui.clone();
        let handler = RcBlock::new(move |color: *mut NSColor| {
            // SAFETY: AppKit passes the selected color, or nil when the person canceled, and keeps
            // it alive while the handler runs.
            let answer: Answer = match unsafe { color.as_ref() } {
                None => Ok(None),
                Some(color) => srgb(color).map(Some).ok_or_else(|| "the sampled color has no sRGB value".into()),
            };
            // The UI may be gone (the app quitting): nothing waits for the color then.
            let _ = tx.send(answer);
            ui.request_repaint();
        });
        let sampler = NSColorSampler::new();
        // SAFETY: on the main thread (checked above); the handler has the signature AppKit calls it
        // with, and AppKit keeps the block and the sampler until the session ends.
        unsafe { sampler.showSamplerWithSelectionHandler(&handler) };
        Ok(rx)
    })
}

/// The components of `color` in sRGB, or `None` when it has no RGB form (a pattern color).
fn srgb(color: &NSColor) -> Option<[f32; 3]> {
    let c = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some([c.redComponent() as f32, c.greenComponent() as f32, c.blueComponent() as f32])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A color in another space arrives as its sRGB components.
    #[test]
    fn sampled_colors_convert_to_srgb() {
        let red = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.0, 0.0, 1.0);
        assert_eq!(srgb(&red), Some([1.0, 0.0, 0.0]));
        // Display P3 (0.6, 0.4, 0.3) lies inside sRGB with other numbers: more red, less green and blue.
        let p3 = NSColor::colorWithDisplayP3Red_green_blue_alpha(0.6, 0.4, 0.3, 1.0);
        let [r, g, b] = srgb(&p3).unwrap();
        assert!(r > 0.61 && g < 0.39 && b < 0.29, "converted, not read as sRGB numbers: {r} {g} {b}");
        // A color outside sRGB is clipped to it.
        assert_eq!(srgb(&NSColor::colorWithDisplayP3Red_green_blue_alpha(1.0, 0.0, 0.0, 1.0)), Some([1.0, 0.0, 0.0]));
        let gray = NSColor::colorWithWhite_alpha(0.5, 1.0);
        let [r, g, b] = srgb(&gray).unwrap();
        assert!((r - g).abs() < 1e-4 && (g - b).abs() < 1e-4, "{r} {g} {b}");
    }
}
