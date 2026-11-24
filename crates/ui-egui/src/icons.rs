//! The app's own icon set, registered beside the framework's catalog, and the map from tool names to icons.

use craft::ui::primitives::{AppIcons, icon_names};

use crate::icon_data::{DC_ARROW_FILL, ICONS};

/// The fills of the Selection and Direct Selection arrows: black and white whatever the theme, as users
/// know them, under an outline in the theme's colour.
const FILLS: &[(&str, &[u8], [u8; 4])] = &[("dc-selection", DC_ARROW_FILL, [0, 0, 0, 255]), ("dc-direct", DC_ARROW_FILL, [255, 255, 255, 255])];

/// The app's icons, for `register_app_icons`.
pub static APP_ICONS: AppIcons = AppIcons::new("vectorcraft", Some(1.6), ICONS).with_fills(FILLS);

/// Map tool-catalogue icon names to the names of icons that resolve.
pub fn tool_icon(name: &str) -> &'static str {
    match name {
        "tool-selection" => "dc-selection",
        "tool-direct" => "dc-direct",
        "tool-group-select" => "dc-group-select",
        "tool-magic-wand" => "wand_sparkles",
        "tool-lasso" => "dc-lasso",
        "tool-lasso-enclosing" => "dc-lasso-enclosing",
        "tool-pen" => "pen-tool",
        "tool-pen-add" => "pen-tool-add",
        "tool-pen-delete" => "pen-tool-delete",
        "tool-anchor" => "dc-anchor",
        "tool-quick-pen" => "dc-quick-pen",
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
        "tool-three-point-arc" => "dc-arc-three-point",
        "tool-spiral" => "tornado",
        "tool-rect-grid" => "dc-rect-grid",
        "tool-polar-grid" => "dc-polar-grid",
        "tool-rect" => "square",
        "tool-rounded-rect" => "dc-rounded-rect",
        "tool-ellipse" => "dc-ellipse",
        "tool-circle" => "dc-circle-points",
        "tool-curvature-circle" => "dc-curvature-circle",
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
        "tool-rotate" => "rotate_ccw",
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
        "tool-fill-area" => "dc-fill-area",
        "tool-live-select" => "dc-live-select",
        "tool-perspective" => "dc-perspective",
        "tool-perspective-select" => "dc-perspective-select",
        "tool-mesh" => "dc-mesh",
        "tool-gradient" => "dc-gradient",
        "tool-eyedropper" => "eyedropper",
        "tool-measure" => "dc-measure",
        "tool-dimension" => "dc-dimension",
        "tool-contour" => "dc-contour",
        "tool-blend" => "dc-blend",
        "tool-objects-on-path" => "dc-objects-on-path",
        "tool-symbol" => "dc-symbol-sprayer",
        "tool-symbol-shifter" => "dc-symbol-shifter",
        "tool-symbol-scruncher" => "dc-symbol-scruncher",
        "tool-symbol-sizer" => "dc-symbol-sizer",
        "tool-symbol-spinner" => "dc-symbol-spinner",
        "tool-symbol-stainer" => "dc-symbol-stainer",
        "tool-symbol-screener" => "dc-symbol-screener",
        "tool-symbol-styler" => "dc-symbol-styler",
        "tool-graph" => "dc-graph-column",
        "tool-graph-stacked-column" => "dc-graph-stacked-column",
        "tool-graph-bar" => "dc-graph-bar",
        "tool-graph-stacked-bar" => "dc-graph-stacked-bar",
        "tool-graph-line" => "dc-graph-line",
        "tool-graph-area" => "dc-graph-area",
        "tool-graph-scatter" => "dc-graph-scatter",
        "tool-graph-pie" => "dc-graph-pie",
        "tool-graph-radar" => "dc-graph-radar",
        "tool-artboard" => "artboard",
        "tool-slice" => "slice_knife",
        "tool-slice-select" => "dc-slice-select",
        "tool-hand" => "hand",
        "tool-rotate-view" => "dc-rotate-view",
        "tool-print-tiling" => "printer",
        "tool-zoom" => "zoom_in",
        // A name that is already an icon maps to itself, as the `'static` name of the set that has it.
        other => icon_names().iter().copied().chain(APP_ICONS.icons.iter().map(|(n, _)| *n)).find(|n| *n == other).unwrap_or("rect_dashed"),
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::primitives::{has_icon, register_app_icons};
    // PLAN: assumed the `craft` facade is a dependency of vectorcraft-ui-egui (Cargo.toml is not in this set's files; craft-ui does not re-export IconId)
    use craft::behavior::IconId;

    use super::*;

    fn resolves(name: &'static str) -> bool {
        register_app_icons(&APP_ICONS);
        has_icon(&IconId::new(name))
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
        let names: Vec<&'static str> = tools.iter().map(|id| tool_icon(vectorcraft_tools::catalog::tool_info(id).unwrap().icon)).collect();
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), tools.len(), "{names:?}");
        for name in &names {
            assert!(name.starts_with("dc-"), "{name} is drawn for VectorCraft");
            assert!(resolves(name), "{name}");
        }
    }

    /// Every cataloged tool, the add-ons' included, maps to an existing icon that is not the
    /// dashed placeholder and that no other tool uses.
    #[test]
    fn every_tool_has_an_icon_no_other_tool_uses() {
        let mut users: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
        for tool in vectorcraft_tools::catalog::all_tools() {
            let name = tool_icon(tool.icon);
            assert!(resolves(name) && name != "rect_dashed", "{} maps to {name}, which isn't an icon of its own", tool.id);
            users.entry(name).or_default().push(tool.id);
        }
        let shared: Vec<_> = users.iter().filter(|(_, tools)| tools.len() > 1).collect();
        assert!(shared.is_empty(), "tools that share an icon: {shared:?}");
    }

    /// An unresolved name draws the fallback square silently, and the registration only logs a refused
    /// icon: this is what notices either.
    #[test]
    fn every_name_tool_icon_returns_resolves_and_no_app_icon_is_refused() {
        for tool in vectorcraft_tools::catalog::all_tools() {
            let name = tool_icon(tool.icon);
            assert!(resolves(name), "{} maps to {name}, which no icon set has", tool.icon);
        }
        let catalog = icon_names();
        for (name, _) in APP_ICONS.icons {
            assert!(!catalog.contains(name), "{name} is in the catalog: the app's copy is refused");
        }
    }
}
