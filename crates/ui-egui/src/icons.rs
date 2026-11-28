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
        "tool-spiral" => "dc-spiral",
        "tool-rect-grid" => "dc-rect-grid",
        "tool-polar-grid" => "dc-polar-grid",
        "tool-rect" => "dc-rect",
        "tool-rounded-rect" => "dc-rounded-rect",
        "tool-ellipse" => "dc-ellipse",
        "tool-circle" => "dc-circle-points",
        "tool-curvature-circle" => "dc-curvature-circle",
        "tool-polygon" => "dc-polygon",
        "tool-star" => "star",
        "tool-flare" => "dc-flare",
        "tool-brush" => "paintbrush",
        "tool-blob-brush" => "dc-blob-brush",
        "tool-shaper" => "dc-shaper",
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
        "tool-rotate" => "dc-rotate",
        "tool-reflect" => "flip-horizontal-2",
        "tool-scale" => "dc-scale",
        "tool-shear" => "dc-shear",
        "tool-reshape" => "dc-reshape",
        "tool-width" => "dc-width",
        "tool-warp" => "dc-warp",
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
        "tool-artboard" => "dc-artboard",
        "tool-slice" => "dc-slice",
        "tool-slice-select" => "dc-slice-select",
        "tool-hand" => "hand",
        "tool-rotate-view" => "dc-rotate-view",
        "tool-print-tiling" => "dc-print-tiling",
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

    /// The ways a source line writes an icon id as a literal; a `const` that holds one is read by `const_icon`.
    const FORMS: [&str; 3] = ["IconId::new(\"", "icon(\"", "icon = \""];

    /// The id a `const NAME: &str = "<id>"` line declares, when `NAME` ends in `ICON` or `code` passes it to
    /// `IconId::new(NAME)`.
    fn const_icon<'a>(line: &'a str, code: &str) -> Option<&'a str> {
        let rest = line.trim_start().trim_start_matches("pub ").trim_start_matches("pub(crate) ").strip_prefix("const ")?;
        let (name, after) = rest.split_once(':')?;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            return None;
        }
        if !name.ends_with("ICON") && !code.contains(&format!("IconId::new({name})")) {
            return None;
        }
        after.trim_start().strip_prefix("&str")?.split_once("= \"")?.1.split('"').next()
    }

    /// An id that no set has draws nothing and raises no error at build time. Every icon id this crate's code
    /// writes as a literal or a `const`, and every icon panel's and dock tab's icon, resolves in the catalog or the
    /// app's set. Other ids built in match arms and tuple tables are not scanned.
    #[test]
    fn every_icon_id_literal_and_panel_icon_resolves() {
        register_app_icons(&APP_ICONS);
        let resolves = |id: &str| has_icon(&IconId::from(std::borrow::Cow::Owned(id.to_owned())));
        let (mut seen, mut unknown) = (0, Vec::new());
        let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
            for entry in entries {
                let path = entry.unwrap_or_else(|e| panic!("{}: {e}", dir.display())).path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.is_dir() {
                    if name != "tests" {
                        stack.push(path);
                    }
                    continue;
                }
                if !name.ends_with(".rs") || name.starts_with("tests") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                // Code up to the test module: a test harness's `icon("..")` names a panel, not an icon.
                let code = text.find("#[cfg(test)]\nmod ").map_or(text.as_str(), |at| &text[..at]);
                for (number, line) in code.lines().enumerate().filter(|(_, l)| !l.trim_start().starts_with("//")) {
                    let literal = |form: &&str| {
                        line.match_indices(*form).filter_map(|(at, form)| line[at + form.len()..].split('"').next()).collect::<Vec<_>>()
                    };
                    for id in FORMS.iter().flat_map(literal).chain(const_icon(line, code)) {
                        seen += 1;
                        if !resolves(id) {
                            unknown.push(format!("{}:{}: {id:?}", path.display(), number + 1));
                        }
                    }
                }
            }
        }
        assert!(seen > 50, "the source scan found only {seen} icon ids");
        let tabs = crate::state::DockTab::ALL.map(crate::state::DockTab::info);
        for (panel, _, icon) in crate::state::ICON_PANELS.iter().chain(tabs.iter()) {
            if !resolves(icon) {
                unknown.push(format!("panel {panel}: {icon:?}"));
            }
        }
        assert!(unknown.is_empty(), "icon ids that no set has: {unknown:#?}");
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

    // Regression test for storytold/vectorcraft#9229: the Stroke panel's three cap icons looked
    // almost the same. The end-of-path line sat at a different x in Round, and the stroke bar was
    // as opaque as the line, so the line vanished where it crossed the bar.
    #[test]
    fn the_cap_icons_mark_the_path_end_in_one_place_over_a_lighter_stroke() {
        let svg = |name: &str| {
            let (_, bytes) = crate::icon_data::ICONS.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name} isn't an icon"));
            String::from_utf8_lossy(bytes).into_owned()
        };
        // The end-of-path line is the icon's last shape, a vertical line `M{x} …V…`.
        let end_line = |icon: &str| -> String {
            let last = icon.rsplit("<path").next().unwrap_or_default();
            last.split('"').nth(1).unwrap_or_default().to_string()
        };
        let stroke_opacity = |icon: &str| -> f32 {
            icon.split(r#"fill-opacity=""#).nth(1).and_then(|s| s.split('"').next()).and_then(|v| v.parse().ok()).unwrap_or(1.0)
        };
        let icons = [("butt", svg("dc-cap-butt")), ("round", svg("dc-cap-round")), ("square", svg("dc-cap-square"))];
        let butt_end = end_line(&icons[0].1);
        assert!(butt_end.starts_with('M') && butt_end.contains('V'), "the butt cap icon ends with a vertical end-of-path line: {butt_end}");
        for (name, icon) in &icons {
            assert_eq!(end_line(icon), butt_end, "the {name} cap icon marks the end of the path where the butt cap icon does");
            assert!(stroke_opacity(icon) < 1.0, "the {name} cap icon's stroke is lighter than its end-of-path line");
        }
        // The stroke bar, everything before the end-of-path line, shows each cap differently.
        let bar = |icon: &str| -> String { icon.rsplit_once("<path").map(|(b, _)| b.to_string()).unwrap_or_default() };
        for (i, (a, icon_a)) in icons.iter().enumerate() {
            for (b, icon_b) in &icons[i + 1..] {
                assert_ne!(bar(icon_a), bar(icon_b), "the {a} and {b} cap icons draw the same stroke");
            }
        }
    }

    /// These tools used general-purpose library icons whose picture did not show the tool (a tornado
    /// for Spiral, waves for Warp, a printer for Print Tiling). Each has an icon drawn for
    /// VectorCraft that no other tool uses.
    #[test]
    fn tools_with_drawn_icons_do_not_fall_back_to_library_icons() {
        let tools = [
            "spiral",
            "rectangle",
            "blobBrush",
            "shaper",
            "rotate",
            "scale",
            "warp",
            "artboard",
            "printTiling",
            "measureInspect",
            "parametricShape",
            "tangentCircle",
        ];
        for id in tools {
            let name = tool_icon(vectorcraft_tools::tool_info(id).unwrap_or_else(|| panic!("{id} is a tool")).icon);
            assert!(name.starts_with("dc-"), "{id} maps to {name}, a library icon");
            assert!(resolves(name), "{id} maps to {name}, which isn't an icon");
            let shared: Vec<&str> =
                vectorcraft_tools::catalog::all_tools().filter(|t| t.id != id && tool_icon(t.icon) == name).map(|t| t.id).collect();
            assert!(shared.is_empty(), "{id} shares {name} with {shared:?}");
        }
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
