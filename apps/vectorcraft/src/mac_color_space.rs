//! macOS: the windows' Metal layers are tagged sRGB, so macOS converts what VectorCraft draws to
//! the display's color profile (#2249, #3161).
//!
//! VectorCraft draws sRGB values. Apple's documentation of `CAMetalLayer.colorspace` says: "When a
//! color space is present, Core Animation performs any necessary color space transformations when
//! compositing this content. The default value is `nil`, indicating that the rendered content
//! isn’t color-matched"
//! (<https://developer.apple.com/documentation/quartzcore/cametallayer/colorspace>). Without a
//! color space, a wide-gamut display shows the sRGB values as its own, so `#FF0000` is drawn as
//! the display's most saturated red, brighter than the same color in an exported PNG.
//!
//! wgpu 30 sets the layer's color space to nil each time it configures the surface: at start, on
//! a resize and when the window moves to a display with another scale (gfx-rs/wgpu#10013). egui
//! configures the surface while it paints a frame, after [`eframe::App::ui`] has run, so [`tag`]
//! adds a paint callback that draws nothing. egui calls it after the configuration and before the
//! frame is presented, and it sets sRGB on every window's Metal layer that has no color space. A
//! layer that already has a color space keeps it.
//!
//! Reading a layer's sublayers has no safe API, hence the scoped `unsafe_code` allowance.
#![allow(unsafe_code)]

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, wgpu};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::{DowncastTarget, Message};
use objc2_app_kit::NSApplication;
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceSRGB};
use objc2_quartz_core::{CALayer, CAMetalLayer};

/// Adds the callback that tags the windows' Metal layers to the frame `ui` draws: a point in its
/// top-left corner, in the background layer, with nothing drawn.
pub fn tag(ui: &egui::Ui) {
    let ctx = ui.ctx();
    let rect = egui::Rect::from_min_size(ctx.content_rect().min, egui::vec2(1.0, 1.0));
    ctx.layer_painter(egui::LayerId::background()).add(egui_wgpu::Callback::new_paint_callback(rect, TagLayers));
}

struct TagLayers;

impl CallbackTrait for TagLayers {
    fn paint(&self, _: egui::PaintCallbackInfo, _: &mut wgpu::RenderPass<'static>, _: &CallbackResources) {
        // AppKit's windows and views are read on the main thread only, where eframe paints.
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        for window in NSApplication::sharedApplication(mtm).windows() {
            if let Some(root) = window.contentView().and_then(|view| view.layer()) {
                for layer in metal_layers(&root) {
                    tag_srgb(&layer);
                }
            }
        }
    }
}

/// The Metal layers that draw a view whose layer is `root`: `root` itself, or the sublayers wgpu
/// adds to it when it is a plain layer.
fn metal_layers(root: &CALayer) -> Vec<Retained<CAMetalLayer>> {
    if let Some(layer) = downcast(root) {
        return vec![layer];
    }
    // SAFETY: the returned array is a copy; the layers in it are retained while it lives.
    let Some(sublayers) = (unsafe { root.sublayers() }) else {
        return Vec::new();
    };
    sublayers.iter().filter_map(|layer| downcast(&layer)).collect()
}

fn downcast<T: DowncastTarget>(layer: &CALayer) -> Option<Retained<T>> {
    layer.retain().downcast::<T>().ok()
}

/// Sets sRGB as `layer`'s color space when it has none. Returns whether it was set.
fn tag_srgb(layer: &CAMetalLayer) -> bool {
    if layer.colorspace().is_some() {
        return false;
    }
    // SAFETY: a constant that CoreGraphics exports.
    let Some(srgb) = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })) else {
        return false;
    };
    layer.setColorspace(Some(&srgb));
    true
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CFString;
    use objc2_core_graphics::kCGColorSpaceDisplayP3;

    use super::*;

    fn space_name(layer: &CAMetalLayer) -> Option<String> {
        CGColorSpace::name(layer.colorspace().as_deref()).map(|name| name.to_string())
    }

    fn name(constant: &CFString) -> String {
        constant.to_string()
    }

    /// A layer without a color space is tagged sRGB once; the second call leaves it as it is.
    #[test]
    fn an_untagged_layer_becomes_srgb() {
        let layer = CAMetalLayer::new();
        assert_eq!(space_name(&layer), None, "a new layer has no color space");
        assert!(tag_srgb(&layer));
        assert_eq!(space_name(&layer), Some(name(unsafe { kCGColorSpaceSRGB })));
        assert!(!tag_srgb(&layer));
        assert_eq!(space_name(&layer), Some(name(unsafe { kCGColorSpaceSRGB })));
    }

    /// A layer with a color space of its own keeps it.
    #[test]
    fn a_layer_with_a_color_space_keeps_it() {
        let layer = CAMetalLayer::new();
        let p3 = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceDisplayP3 })).unwrap();
        layer.setColorspace(Some(&p3));
        assert!(!tag_srgb(&layer));
        assert_eq!(space_name(&layer), Some(name(unsafe { kCGColorSpaceDisplayP3 })));
    }

    /// The Metal layer is found as the view's layer or as one of its sublayers, and only Metal
    /// layers are returned.
    #[test]
    fn metal_layers_are_found_under_the_view_layer() {
        let metal = CAMetalLayer::new();
        assert_eq!(metal_layers(&metal), vec![metal.clone()]);

        let root = CALayer::new();
        assert!(metal_layers(&root).is_empty());
        root.addSublayer(&CALayer::new());
        root.addSublayer(&metal);
        assert_eq!(metal_layers(&root), vec![metal.clone()]);
        for layer in metal_layers(&root) {
            tag_srgb(&layer);
        }
        assert_eq!(space_name(&metal), Some(name(unsafe { kCGColorSpaceSRGB })));
    }
}
