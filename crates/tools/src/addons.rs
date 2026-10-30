//! The tools of VectorCraft's built-in add-ons. They aren't in the toolbars as they ship: the All
//! Tools drawer lists them (under their add-on's heading) so they can be added, and each add-on's
//! submenu selects them. Like every tool they only emit commands (`addon.<name>.*`).

pub mod anchor_eraser;
pub mod extend_path;
pub mod measure_inspect;
pub mod smart_pen;

use crate::{Tool, ToolInfo};

/// Path Precision Tools' tools, one flyout group.
pub const PATH_TOOLS: &[ToolInfo] = &[
    ToolInfo { id: anchor_eraser::ID, label: "Anchor Eraser Brush", shortcut: None, icon: "dc-remove-brush" },
    ToolInfo { id: extend_path::ID, label: "Extend Path Tool", shortcut: None, icon: "dc-arrow-extend" },
    ToolInfo { id: measure_inspect::ID, label: "Measure & Inspect Tool", shortcut: None, icon: "ruler" },
];

/// Smart Pen's tool, a flyout group of its own.
pub const SMART_PEN: &[ToolInfo] = &[ToolInfo { id: smart_pen::ID, label: "Smart Pen Tool", shortcut: None, icon: "spline-pointer" }];

/// The add-ons' tool groups, after the toolbar's own ([`crate::TOOL_GROUPS`]), each with the
/// heading the All Tools drawer lists it under (its add-on's name).
const HEADED: &[(&[ToolInfo], &str)] = &[(PATH_TOOLS, "Path Precision Tools"), (SMART_PEN, "Smart Pen")];

/// The add-ons' tool groups, after the toolbar's own ([`crate::TOOL_GROUPS`]).
pub const GROUPS: &[&[ToolInfo]] = &[PATH_TOOLS, SMART_PEN];

/// The heading the All Tools drawer lists tool `id` under: its add-on's name.
pub fn heading(id: &str) -> Option<&'static str> {
    HEADED.iter().find(|(g, _)| g.iter().any(|t| t.id == id)).map(|(_, h)| *h)
}

/// Create an add-on tool by id (None = not one of them).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        anchor_eraser::ID => Box::new(anchor_eraser::AnchorEraserTool::default()),
        extend_path::ID => Box::new(extend_path::ExtendPathTool::default()),
        measure_inspect::ID => Box::new(measure_inspect::MeasureInspectTool::default()),
        smart_pen::ID => Box::new(smart_pen::SmartPenTool::default()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_addon_tool_is_made_and_catalogued_but_not_in_the_toolbar() {
        for t in GROUPS.iter().flat_map(|g| g.iter()) {
            assert_eq!(create(t.id).map(|tool| tool.id()), Some(t.id));
            assert_eq!(crate::tool_info(t.id).map(|i| i.label), Some(t.label));
            assert!(crate::create(t.id).id() == t.id);
            assert!(t.shortcut.is_none(), "no default shortcut");
            assert!(!crate::TOOL_GROUPS.iter().any(|g| g.iter().any(|x| x.id == t.id)), "{} is not in the toolbar's groups", t.id);
        }
        assert!(PATH_TOOLS.iter().all(|t| heading(t.id) == Some("Path Precision Tools")));
        assert_eq!(heading(smart_pen::ID), Some("Smart Pen"));
        assert_eq!(GROUPS.len(), HEADED.len());
        assert!(crate::catalog::all_tools().any(|t| t.id == extend_path::ID));
        assert!(create("pen").is_none() && heading("pen").is_none());
    }
}
