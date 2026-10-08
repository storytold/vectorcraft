//! Icon lookup and drawing (SVGs rasterized by egui_extras, tinted with theme colours).

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{Color32, ImageSource, Rect, Ui, Vec2};

fn table() -> &'static HashMap<&'static str, &'static [u8]> {
    static T: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    T.get_or_init(|| {
        crate::icon_data::ICONS
            .iter()
            .map(|(n, b)| {
                // Icons use currentColor; render white and tint at draw time. Slightly thinner strokes
                // than Lucide's default read closer to Illustrator's glyph weight.
                let s = String::from_utf8_lossy(b).replace("currentColor", "white").replace("stroke-width=\"2\"", "stroke-width=\"1.6\"");
                let leaked: &'static [u8] = Box::leak(s.into_bytes().into_boxed_slice());
                (*n, leaked)
            })
            .collect()
    })
}

pub fn exists(name: &str) -> bool {
    table().contains_key(name)
}

/// Map tool-catalogue icon names to SVG files.
pub fn tool_icon(name: &str) -> &'static str {
    match name {
        "tool-selection" => "dc-selection",
        "tool-direct" => "dc-direct",
        "tool-group-select" => "dc-group-select",
        "tool-magic-wand" => "wand-sparkles",
        "tool-lasso" => "lasso",
        "tool-pen" => "pen-tool",
        "tool-pen-add" => "dc-pen-add",
        "tool-pen-delete" => "dc-pen-delete",
        "tool-anchor" => "dc-anchor",
        "tool-curvature" => "spline",
        "tool-type" => "type",
        "tool-type-area" => "dc-type-area",
        "tool-type-path" => "dc-type-path",
        "tool-type-vertical" => "dc-type-vertical",
        "tool-touch-type" => "dc-touch-type",
        "tool-line" => "dc-line",
        "tool-arc" => "dc-arc",
        "tool-spiral" => "tornado",
        "tool-rect-grid" => "dc-rect-grid",
        "tool-polar-grid" => "dc-polar-grid",
        "tool-rect" => "square",
        "tool-rounded-rect" => "dc-rounded-rect",
        "tool-ellipse" => "dc-ellipse",
        "tool-polygon" => "dc-polygon",
        "tool-star" => "star",
        "tool-flare" => "dc-flare",
        "tool-brush" => "paintbrush",
        "tool-blob-brush" => "brush",
        "tool-shaper" => "shapes",
        "tool-pencil" => "pencil",
        "tool-smooth" => "dc-smooth",
        "tool-path-eraser" => "dc-path-eraser",
        "tool-join" => "dc-join",
        "tool-eraser" => "eraser",
        "tool-scissors" => "scissors",
        "tool-knife" => "dc-knife",
        "tool-mirror-cut" => "dc-mirror-cut",
        "tool-line-cut" => "dc-line-cut",
        "tool-rect-cut" => "dc-rect-cut",
        "tool-rotate" => "rotate-ccw",
        "tool-reflect" => "flip-horizontal-2",
        "tool-scale" => "scaling",
        "tool-shear" => "dc-shear",
        "tool-reshape" => "dc-reshape",
        "tool-width" => "dc-width",
        "tool-warp" => "waves",
        "tool-twirl" => "dc-twirl",
        "tool-pucker" => "dc-pucker",
        "tool-bloat" => "dc-bloat",
        "tool-scallop" => "dc-scallop",
        "tool-crystallize" => "dc-crystallize",
        "tool-wrinkle" => "dc-wrinkle",
        "tool-free-transform" => "dc-free-transform",
        "tool-puppet" => "dc-puppet",
        "tool-shape-builder" => "dc-shape-builder",
        "tool-bucket" => "dc-live-bucket",
        "tool-live-select" => "dc-live-select",
        "tool-perspective" => "dc-perspective",
        "tool-perspective-select" => "dc-perspective",
        "tool-mesh" => "dc-mesh",
        "tool-gradient" => "dc-gradient",
        "tool-eyedropper" => "pipette",
        "tool-measure" => "dc-measure",
        "tool-blend" => "dc-blend",
        "tool-symbol" => "dc-symbol-sprayer",
        "tool-graph" => "chart-column",
        "tool-artboard" => "frame",
        "tool-slice" => "slice",
        "tool-hand" => "hand",
        "tool-rotate-view" => "dc-rotate-view",
        "tool-print-tiling" => "printer",
        "tool-zoom" => "zoom-in",
        other => {
            if exists(other) {
                table().get_key_value(other).map(|(k, _)| *k).unwrap_or("square-dashed")
            } else {
                "square-dashed"
            }
        }
    }
}

pub fn source(name: &str) -> ImageSource<'static> {
    let bytes = table().get(name).or_else(|| table().get("square-dashed")).copied().unwrap_or(&[]);
    ImageSource::Bytes { uri: format!("bytes://icon/{name}.svg").into(), bytes: egui::load::Bytes::Static(bytes) }
}

/// Icons drawn in their own colours, whatever the theme: the Selection tool's black arrow and the
/// Direct Selection tool's white arrow, which are told apart by colour.
const TWO_TONE: [&str; 2] = ["dc-selection", "dc-direct"];

/// Paint icon `name` into `rect` tinted with `tint` (two-tone icons keep their colours).
pub fn paint(ui: &Ui, name: &str, rect: Rect, tint: Color32) {
    let tint = if TWO_TONE.contains(&name) { Color32::WHITE } else { tint };
    egui::Image::new(source(name)).tint(tint).fit_to_exact_size(rect.size()).paint_at(ui, rect);
}

/// An icon widget of `size` points.
pub fn icon(ui: &mut Ui, name: &str, size: f32, tint: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint(ui, name, rect, tint);
    resp
}
