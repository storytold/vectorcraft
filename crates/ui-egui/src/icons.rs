//! Icon lookup and drawing (SVGs rasterized by egui_extras, tinted with theme colours).

use std::sync::OnceLock;

use craft_ui::icons::{SvgIconSet, SvgStyle};
use egui::{Color32, ImageSource, Rect, Ui, Vec2};

fn quiet_table() -> &'static SvgIconSet {
    static T: OnceLock<SvgIconSet> = OnceLock::new();
    T.get_or_init(|| {
        SvgIconSet::new(
            crate::icon_data_quiet::ICONS,
            SvgStyle { current_color: "white", stroke_width: None },
            "bytes://icon-quiet/",
            "square-dashed",
        )
    })
}

/// Tool-catalogue icon names whose Quiet icon is a drawing of its own, not the one the name maps
/// to in the shared set (which reuses it for other tools).
fn quiet_role(name: &str) -> Option<&'static str> {
    Some(match name {
        "tool-perspective-select" => "proposal-perspective-select",
        "tool-symbol" => "proposal-symbol-style",
        "tool-graph" => "proposal-graph-radar",
        "tool-slice" => "proposal-slice-select",
        "dc-rounded-rect" => "proposal-corner-tool",
        "dc-anchor" => "proposal-slide-anchor",
        "dc-join" => "proposal-shared-edge",
        "box" => "proposal-box3d",
        _ => return None,
    })
}

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
        "tool-type" => "dc-type",
        "tool-type-area" => "dc-type-area",
        "tool-type-path" => "dc-type-path",
        "tool-type-vertical" => "dc-type-vertical",
        "tool-type-vertical-area" => "dc-type-vertical-area",
        "tool-type-vertical-path" => "dc-type-vertical-path",
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
        "tool-contour" => "dc-contour",
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

/// Whether `name` is drawn from the Quiet icon set: Quiet is on and the set has the icon.
fn in_quiet_set(ctx: &egui::Context, name: &str) -> bool {
    crate::theme::Tokens::get(ctx).quiet_icons && quiet_table().contains(name)
}

/// The image for icon `name` under the colour scheme in force in `ctx`: Quiet's own drawing where
/// it has one, the shared icon elsewhere.
pub fn source(ctx: &egui::Context, name: &str) -> ImageSource<'static> {
    if in_quiet_set(ctx, name) { quiet_table().source(name) } else { table().source(name) }
}

/// [`tool_icon`] under the colour scheme in force in `ctx`: Quiet gives a few tools an icon of
/// their own where the shared set reuses one for several.
pub fn tool_icon_for(ctx: &egui::Context, name: &str) -> &'static str {
    match quiet_role(name) {
        Some(icon) if crate::theme::Tokens::get(ctx).quiet_icons => icon,
        _ => tool_icon(name),
    }
}

/// Paint icon `name` into `rect` tinted with `tint`. The Selection and Direct Selection arrows are
/// black and white whatever the theme, as users know them (a light theme colour would make the
/// Selection arrow look like Direct Selection's): their fill is painted first, then their outline
/// in `tint`.
pub fn paint(ui: &Ui, name: &str, rect: Rect, tint: Color32) {
    // Quiet's own arrows are drawn to stand alone: no fill underneath.
    let fill = match name {
        "dc-selection" if !in_quiet_set(ui.ctx(), name) => Some(Color32::BLACK),
        "dc-direct" if !in_quiet_set(ui.ctx(), name) => Some(Color32::WHITE),
        _ => None,
    };
    if let Some(fill) = fill {
        egui::Image::new(source(ui.ctx(), "dc-arrow-fill")).tint(fill).fit_to_exact_size(rect.size()).paint_at(ui, rect);
    }
    egui::Image::new(source(ui.ctx(), name)).tint(tint).fit_to_exact_size(rect.size()).paint_at(ui, rect);
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
            let ImageSource::Bytes { uri, bytes } = source(&egui::Context::default(), name) else { panic!("expected SVG bytes") };
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

    // Regression test for #479: the Add and Delete Anchor Point tools' icons were drawn apart from
    // the Pen tool's, so the three tools didn't read as one family. They are the Pen's pen with a
    // plus or a minus beside it.
    #[test]
    fn the_add_and_delete_anchor_tool_icons_are_the_pen_icon_with_a_badge() {
        let svg = |tool: &str| {
            let name = tool_icon(tool);
            let (_, bytes) =
                crate::icon_data::ICONS.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{tool} maps to {name}, which isn't an icon"));
            String::from_utf8_lossy(bytes).into_owned()
        };
        let shapes =
            |svg: &str| svg.lines().map(|l| l.trim().to_string()).filter(|l| l.starts_with("<path") || l.starts_with("<circle")).collect::<Vec<_>>();
        let pen = shapes(&svg("tool-pen"));
        assert!(!pen.is_empty(), "the Pen's icon has shapes");
        let (add, delete) = (shapes(&svg("tool-pen-add")), shapes(&svg("tool-pen-delete")));
        for (tool, icon) in [("tool-pen-add", &add), ("tool-pen-delete", &delete)] {
            for shape in &pen {
                assert!(icon.contains(shape), "{tool}'s icon lacks the Pen's {shape}");
            }
            assert!(icon.len() > pen.len(), "{tool}'s icon has a badge of its own");
        }
        assert_ne!(add, delete, "plus and minus differ");
    }

    /// The seven type tools each have an icon of their own, drawn for VectorCraft (the vertical
    /// Area and Path tools used to borrow the horizontal ones' icons).
    #[test]
    fn every_type_tool_has_its_own_icon_with_the_same_t() {
        let tools = ["type", "areaType", "typeOnPath", "verticalType", "verticalAreaType", "verticalTypeOnPath", "touchType"];
        let names: Vec<&str> = tools.iter().map(|id| tool_icon(vectorcraft_tools::catalog::tool_info(id).unwrap().icon)).collect();
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), tools.len(), "{names:?}");
        for name in &names {
            assert!(name.starts_with("dc-"), "{name} is drawn for VectorCraft");
            assert!(exists(name), "{name}");
        }
    }

    #[test]
    fn shared_catalog_preserves_fallback_and_tool_aliases() {
        let ImageSource::Bytes { uri, bytes } = source(&egui::Context::default(), "missing-icon") else { panic!("expected SVG bytes") };
        let ImageSource::Bytes { bytes: fallback, .. } = source(&egui::Context::default(), "square-dashed") else { panic!("expected SVG bytes") };
        assert_eq!(uri.as_ref(), "bytes://icon/missing-icon.svg");
        assert_eq!(bytes.as_ref(), fallback.as_ref());
        assert!(!exists("missing-icon"));
        assert_eq!(tool_icon("missing-icon"), "square-dashed");
        assert_eq!(tool_icon("tool-selection"), "dc-selection");
    }
    /// A context drawing in colour scheme `scheme`.
    fn ctx_in(scheme: crate::theme::Scheme) -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::apply_with(&ctx, crate::theme::Brightness::MediumDark, crate::theme::Accent::default(), scheme);
        ctx
    }

    fn bytes_of(source: ImageSource<'static>) -> (String, Vec<u8>) {
        let ImageSource::Bytes { uri, bytes } = source else { panic!("expected SVG bytes") };
        (uri.to_string(), bytes.as_ref().to_vec())
    }

    /// Modern, Classic and Flat draw the shared icons, byte for byte, as before the Quiet set.
    #[test]
    fn the_default_and_other_schemes_draw_the_shared_icons() {
        use crate::theme::Scheme;
        for scheme in [Scheme::default(), Scheme::Classic, Scheme::Flat] {
            let ctx = ctx_in(scheme);
            for (name, _) in crate::icon_data::ICONS {
                assert_eq!(bytes_of(source(&ctx, name)), bytes_of(table().source(name)), "{scheme:?} {name}");
            }
            for tool in vectorcraft_tools::catalog::all_tools() {
                assert_eq!(tool_icon_for(&ctx, tool.icon), tool_icon(tool.icon), "{scheme:?} {}", tool.id);
            }
        }
    }

    /// Choosing Quiet switches the icons it has a drawing for, falls back to the shared icon for
    /// the rest, and gives the tools whose icon the shared set reuses an icon each.
    #[test]
    fn quiet_draws_its_own_icons_and_falls_back_to_the_shared_ones() {
        let (quiet, shared) = (ctx_in(crate::theme::Scheme::Quiet), ctx_in(crate::theme::Scheme::default()));
        for (name, _) in crate::icon_data_quiet::ICONS {
            if exists(name) {
                let (uri, svg) = bytes_of(source(&quiet, name));
                assert_eq!(uri, format!("bytes://icon-quiet/{name}.svg"));
                assert_ne!(svg, bytes_of(source(&shared, name)).1, "{name}: Quiet's drawing differs from the shared one");
            }
        }
        let without =
            crate::icon_data::ICONS.iter().map(|(n, _)| *n).find(|n| !quiet_table().contains(n)).expect("some icon isn't in the Quiet set yet");
        assert_eq!(bytes_of(source(&quiet, without)), bytes_of(source(&shared, without)), "{without} falls back");
        assert_ne!(tool_icon_for(&quiet, "tool-graph"), tool_icon_for(&shared, "tool-graph"));
    }

    /// Every tool in the catalogue has an icon in both sets: the name resolves to a drawing in
    /// the Quiet set or in the shared one, never the dashed-square fallback.
    #[test]
    fn every_tool_resolves_to_an_icon_in_every_scheme() {
        for scheme in crate::theme::Scheme::ALL {
            let ctx = ctx_in(scheme);
            for tool in vectorcraft_tools::catalog::all_tools() {
                let name = tool_icon_for(&ctx, tool.icon);
                assert!(exists(name) || in_quiet_set(&ctx, name), "{scheme:?}: {} -> {name}", tool.id);
                assert!(name != "square-dashed" || tool_icon(tool.icon) == "square-dashed", "{scheme:?}: {} fell back to the dashed square", tool.id);
            }
        }
    }
}
