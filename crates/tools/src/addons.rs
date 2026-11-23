//! The tools of VectorCraft's built-in add-ons. They aren't in the toolbars as they ship: the All
//! Tools drawer lists them (under their add-on's heading) so they can be added, and each add-on's
//! submenu selects them. Tool gestures emit engine commands. Two tools are the exceptions: the Trim
//! at Crossings Tool is in the Scissors tool's group and the Enclosing Lasso Tool is in the Lasso
//! tool's group ([`crate::TOOL_GROUPS`]), where the toolbars and the All Tools drawer show them, as
//! well as in their add-ons' submenus.

pub mod anchor_eraser;
pub mod box3d;
pub mod contour;
pub mod corner_tool;
pub mod enclosing_lasso;
pub mod extend_path;
pub mod measure_inspect;
pub mod parametric_shape;
pub mod polygonal_lasso;
pub mod shared_edge;
pub mod slide_anchor;
pub mod smart_pen;
pub mod tangent_circle;
pub mod trim;

use crate::{Tool, ToolInfo};

/// Path Precision Tools' tools, one flyout group. Its Trim at Crossings Tool ([`trim::ID`]) is in
/// the Scissors tool's group instead.
pub const PATH_TOOLS: &[ToolInfo] = &[
    ToolInfo { id: anchor_eraser::ID, label: "Anchor Eraser Brush", shortcut: None, icon: "dc-remove-brush" },
    ToolInfo { id: extend_path::ID, label: "Extend Path Tool", shortcut: None, icon: "dc-arrow-extend" },
    ToolInfo { id: measure_inspect::ID, label: "Measure & Inspect Tool", shortcut: None, icon: "ruler" },
    ToolInfo { id: parametric_shape::ID, label: "Parametric Shape Tool", shortcut: None, icon: "hexagon" },
    ToolInfo { id: corner_tool::ID, label: "Corner Tool", shortcut: None, icon: "dc-rounded-rect" },
    ToolInfo { id: slide_anchor::ID, label: "Slide Anchor Along Path", shortcut: None, icon: "dc-anchor" },
    ToolInfo { id: shared_edge::ID, label: "Shared Edge Merge Tool", shortcut: None, icon: "dc-join" },
    ToolInfo { id: tangent_circle::ID, label: "Tangent Circle Tool", shortcut: None, icon: "circle" },
];

/// Smart Pen's tool, a flyout group of its own.
pub const SMART_PEN: &[ToolInfo] = &[ToolInfo { id: smart_pen::ID, label: "Smart Pen Tool", shortcut: None, icon: "spline-pointer" }];

pub const CONTOUR: &[ToolInfo] = &[ToolInfo { id: contour::ID, label: "Contour Tool", shortcut: None, icon: "tool-contour" }];

pub const BOX_3D: &[ToolInfo] = &[ToolInfo { id: box3d::ID, label: "3D Box Tool", shortcut: None, icon: "box" }];

pub const POLYGONAL_LASSO: &[ToolInfo] = &[ToolInfo { id: polygonal_lasso::ID, label: "Polygonal Lasso Tool", shortcut: None, icon: "tool-lasso" }];

/// The add-ons' tool groups, after the toolbar's own ([`crate::TOOL_GROUPS`]), each with the
/// heading the All Tools drawer lists it under (its add-on's name).
const HEADED: &[(&[ToolInfo], &str)] =
    &[(PATH_TOOLS, "Path Precision Tools"), (SMART_PEN, "Smart Pen"), (CONTOUR, "Contour"), (BOX_3D, "3D Box"), (POLYGONAL_LASSO, "Polygonal Lasso")];

/// The add-ons' tool groups, after the toolbar's own ([`crate::TOOL_GROUPS`]).
pub const GROUPS: &[&[ToolInfo]] = &[PATH_TOOLS, SMART_PEN, CONTOUR, BOX_3D, POLYGONAL_LASSO];

/// The heading the All Tools drawer lists tool `id` under: its add-on's name.
pub fn heading(id: &str) -> Option<&'static str> {
    HEADED.iter().find(|(g, _)| g.iter().any(|t| t.id == id)).map(|(_, h)| *h)
}

/// Create an add-on tool by id (None = not one of them).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        anchor_eraser::ID => Box::new(anchor_eraser::AnchorEraserTool::default()),
        corner_tool::ID => Box::new(corner_tool::CornerTool::default()),
        extend_path::ID => Box::new(extend_path::ExtendPathTool::default()),
        measure_inspect::ID => Box::new(measure_inspect::MeasureInspectTool::default()),
        parametric_shape::ID => Box::new(parametric_shape::ParametricShapeTool::default()),
        shared_edge::ID => Box::new(shared_edge::SharedEdgeTool::default()),
        slide_anchor::ID => Box::new(slide_anchor::SlideAnchorTool::default()),
        smart_pen::ID => Box::new(smart_pen::SmartPenTool::default()),
        tangent_circle::ID => Box::new(tangent_circle::TangentCircleTool::default()),
        trim::ID => Box::new(trim::TrimTool::default()),
        enclosing_lasso::ID => Box::new(enclosing_lasso::EnclosingLassoTool::default()),
        contour::ID => Box::new(contour::ContourTool::default()),
        box3d::ID => Box::new(box3d::Box3dTool::default()),
        polygonal_lasso::ID => Box::new(polygonal_lasso::PolygonalLassoTool::default()),
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
        assert_eq!(heading(contour::ID), Some("Contour"));
        assert_eq!(heading(box3d::ID), Some("3D Box"));
        assert_eq!(heading(polygonal_lasso::ID), Some("Polygonal Lasso"));
        assert_eq!(GROUPS.len(), HEADED.len());
        assert!(crate::catalog::all_tools().any(|t| t.id == extend_path::ID));
        assert!(create("pen").is_none() && heading("pen").is_none());
    }

    /// The Trim at Crossings Tool is the add-on tool in a built-in group: the Scissors tool's,
    /// after the Knife, with no default shortcut, listed once, and still made by its add-on.
    #[test]
    fn trim_at_crossings_is_in_the_scissors_group_once() {
        let groups: Vec<_> = crate::TOOL_GROUPS.iter().filter(|g| g.iter().any(|t| t.id == trim::ID)).collect();
        assert_eq!(groups.len(), 1);
        let ids: Vec<_> = groups[0].iter().map(|t| t.id).collect();
        let knife = ids.iter().position(|id| *id == "knife").expect("the Knife's group");
        assert!(ids.contains(&"scissors"));
        assert_eq!(ids.get(knife + 1), Some(&trim::ID));
        let info = crate::tool_info(trim::ID).expect("catalogued");
        assert_eq!((info.label, info.shortcut, info.icon), ("Trim at Crossings Tool", None, "dc-pf-trim"));
        assert_eq!(create(trim::ID).map(|tool| tool.id()), Some(trim::ID));
        assert!(!GROUPS.iter().any(|g| g.iter().any(|t| t.id == trim::ID)), "not listed again under the add-on");
        assert_eq!(crate::catalog::all_tools().filter(|t| t.id == trim::ID).count(), 1);
    }

    /// The Enclosing Lasso Tool is in the Lasso tool's group, right after the Lasso, with no
    /// default shortcut, listed once, and still made by its add-on.
    #[test]
    fn enclosing_lasso_is_in_the_lasso_group_once() {
        let id = enclosing_lasso::ID;
        let groups: Vec<_> = crate::TOOL_GROUPS.iter().filter(|g| g.iter().any(|t| t.id == id)).collect();
        assert_eq!(groups.len(), 1);
        let ids: Vec<_> = groups[0].iter().map(|t| t.id).collect();
        assert_eq!(ids, ["lasso", id]);
        let info = crate::tool_info(id).expect("catalogued");
        assert_eq!((info.label, info.shortcut, info.icon), ("Enclosing Lasso Tool", None, "tool-lasso"));
        assert_eq!(crate::tool_info("lasso").and_then(|t| t.shortcut), Some("Q"), "the Lasso keeps its shortcut");
        assert_eq!(create(id).map(|tool| tool.id()), Some(id));
        assert!(heading(id).is_none(), "not listed again under the add-on");
        assert_eq!(crate::catalog::all_tools().filter(|t| t.id == id).count(), 1);
    }
}
