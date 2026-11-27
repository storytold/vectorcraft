//! The module defines VectorCraft's built-in add-on menus and panels. Object › Plug-ins lists their tools and commands.

pub mod arrange;
pub mod cleanup;
pub mod color;
pub mod compounds;
pub mod contour;
pub mod cutting;
pub(crate) mod dialogs;
pub mod document;
pub mod enclosing_lasso;
pub mod image;
pub mod packaging;
pub mod pathtools;
pub mod pen;
pub mod polygonal_lasso;
pub mod select;
pub mod shapes;
pub mod typetools;
pub mod units;

use craft::menu::Node;

use crate::menus::{Sep, sub};

/// The add-on panels' state, which lasts for the run and is not saved with a document.
#[derive(Default)]
pub(crate) struct Panels {
    /// The last Artwork Cleanup scan, shown until the next.
    pub cleanup: Option<cleanup::Results>,
    /// The colour the Color Effects panel last picked.
    pub color: color::Picked,
    /// The fold lines of the Packaging Tools panel.
    pub fold: packaging::FoldLines,
    /// The Packaging Tools preview's view.
    pub fold_view: packaging::preview::View,
    /// The Anchor Inspector panel's settings.
    pub inspector: pathtools::Inspector,
    /// The Unit Converter panel's values.
    pub units: units::Converter,
}

/// The add-ons' submenus, then a separator: the head of Object › Plug-ins.
pub fn plugin_menu() -> Vec<Node> {
    vec![
        sub("3D Box", shapes::menu()),
        sub("Arrange Tools", arrange::menu()),
        sub("Artwork Cleanup", cleanup::menu()),
        sub("Color Effects", color::menu()),
        sub("Compound Shapes", compounds::menu()),
        sub("Contour", contour::menu()),
        sub("Cutting Tools", cutting::menu()),
        sub("Document Tools", document::menu()),
        sub("Enclosing Lasso", enclosing_lasso::menu()),
        sub("Image Tools", image::menu()),
        sub("Packaging Tools", packaging::menu()),
        sub("Path Precision Tools", pathtools::menu()),
        sub("Polygonal Lasso", polygonal_lasso::menu()),
        sub("Selection Tools", select::menu()),
        sub("Smart Pen", pen::menu()),
        sub("Type Tools", typetools::menu()),
        sub("Unit Converter", units::menu()),
        Sep,
    ]
}
