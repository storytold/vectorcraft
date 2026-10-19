//! Icon lookup and drawing (SVGs rasterized by egui_extras, tinted with theme colours).

use std::sync::OnceLock;

use craft_ui::icons::{SvgIconSet, SvgStyle};
use egui::{Color32, ImageSource, Rect, Ui, Vec2};

fn table() -> &'static SvgIconSet {
    static T: OnceLock<SvgIconSet> = OnceLock::new();
    T.get_or_init(|| {
        SvgIconSet::new(crate::icon_data::ICONS, SvgStyle { current_color: "white", stroke_width: Some("1.6") }, "bytes://icon/", "square-dashed")
    })
}

pub fn exists(name: &str) -> bool {
    table().contains(name)
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
        "tool-pen-add" => "pen-tool-add",
        "tool-pen-delete" => "pen-tool-delete",
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
        "tool-dimension" => "dc-dimension",
        "tool-blend" => "dc-blend",
        "tool-objects-on-path" => "dc-objects-on-path",
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
                table().canonical_name(other).unwrap_or("square-dashed")
            } else {
                "square-dashed"
            }
        }
    }
}

pub fn source(name: &str) -> ImageSource<'static> {
    table().source(name)
}

/// Paint icon `name` into `rect` tinted with `tint`. The Selection and Direct Selection arrows are
/// black and white whatever the theme, as users know them (a light theme colour would make the
/// Selection arrow look like Direct Selection's): their fill is painted first, then their outline
/// in `tint`.
pub fn paint(ui: &Ui, name: &str, rect: Rect, tint: Color32) {
    let fill = match name {
        "dc-selection" => Some(Color32::BLACK),
        "dc-direct" => Some(Color32::WHITE),
        _ => None,
    };
    if let Some(fill) = fill {
        egui::Image::new(source("dc-arrow-fill")).tint(fill).fit_to_exact_size(rect.size()).paint_at(ui, rect);
    }
    egui::Image::new(source(name)).tint(tint).fit_to_exact_size(rect.size()).paint_at(ui, rect);
}

/// An icon widget of `size` points.
pub fn icon(ui: &mut Ui, name: &str, size: f32, tint: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint(ui, name, rect, tint);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tints of the images `name` paints, bottom first.
    fn tints(name: &str, tint: Color32) -> Vec<Color32> {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let draw = |ui: &mut Ui| paint(ui, name, Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(24.0)), tint);
        // The first frame loads the images; the second draws them.
        ctx.run_ui(egui::RawInput::default(), draw).textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput::default(), draw);
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Mesh(m) => m.vertices.first().map(|v| v.color),
                egui::Shape::Rect(r) if r.brush.is_some() => Some(r.fill),
                _ => None,
            })
            .collect()
    }

    /// The Selection arrow is black and Direct Selection's white in every theme (a reported swap
    /// in the dark themes), under an outline in the theme's colour; other icons are one layer.
    #[test]
    fn the_selection_arrows_are_black_and_white_under_the_theme_outline() {
        let light = Color32::from_gray(220);
        assert_eq!(tints("dc-selection", light), [Color32::BLACK, light]);
        assert_eq!(tints("dc-direct", light), [Color32::WHITE, light]);
        assert_eq!(tints("dc-group-select", light), [light]);
    }

    /// Characterize the pre-extraction loader against every embedded asset, including custom
    /// icons. A shared-crate update must not silently change our stroke weight or cache keys.
    #[test]
    fn shared_catalog_preserves_existing_icons() {
        for (name, original) in crate::icon_data::ICONS {
            let expected = String::from_utf8_lossy(original).replace("currentColor", "white").replace("stroke-width=\"2\"", "stroke-width=\"1.6\"");
            let ImageSource::Bytes { uri, bytes } = source(name) else { panic!("expected SVG bytes") };
            assert_eq!(uri.as_ref(), format!("bytes://icon/{name}.svg"));
            assert_eq!(bytes.as_ref(), expected.as_bytes(), "{name}");
            assert!(exists(name));
            assert_eq!(tool_icon(name), *name);
        }
    }

    /// Appearance (effects) and Pathfinder both lived in the icon column; they used the same
    /// overlapping-squares motif (storytold/vectorcraft#2681).
    #[test]
    fn appearance_panel_icon_is_not_the_pathfinder_icon() {
        let appearance = String::from_utf8_lossy(include_bytes!("../../../assets/icons/dc-appearance.svg"));
        let pathfinder = String::from_utf8_lossy(include_bytes!("../../../assets/icons/dc-pathfinder.svg"));
        assert_ne!(appearance.as_bytes(), pathfinder.as_bytes());
        assert!(appearance.contains("<circle") && !pathfinder.contains("<circle"), "appearance should read as a list, not overlapping squares");
    }

    #[test]
    fn shared_catalog_preserves_fallback_and_tool_aliases() {
        let ImageSource::Bytes { uri, bytes } = source("missing-icon") else { panic!("expected SVG bytes") };
        let ImageSource::Bytes { bytes: fallback, .. } = source("square-dashed") else { panic!("expected SVG bytes") };
        assert_eq!(uri.as_ref(), "bytes://icon/missing-icon.svg");
        assert_eq!(bytes.as_ref(), fallback.as_ref());
        assert!(!exists("missing-icon"));
        assert_eq!(tool_icon("missing-icon"), "square-dashed");
        assert_eq!(tool_icon("tool-selection"), "dc-selection");
    }
}
