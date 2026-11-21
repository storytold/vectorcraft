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

use crate::menus::Item;

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
pub fn plugin_menu() -> Vec<Item> {
    vec![
        Item::Sub("3D Box", shapes::menu()),
        Item::Sub("Arrange Tools", arrange::menu()),
        Item::Sub("Artwork Cleanup", cleanup::menu()),
        Item::Sub("Color Effects", color::menu()),
        Item::Sub("Compound Shapes", compounds::menu()),
        Item::Sub("Contour", contour::menu()),
        Item::Sub("Cutting Tools", cutting::menu()),
        Item::Sub("Document Tools", document::menu()),
        Item::Sub("Enclosing Lasso", enclosing_lasso::menu()),
        Item::Sub("Image Tools", image::menu()),
        Item::Sub("Packaging Tools", packaging::menu()),
        Item::Sub("Path Precision Tools", pathtools::menu()),
        Item::Sub("Polygonal Lasso", polygonal_lasso::menu()),
        Item::Sub("Selection Tools", select::menu()),
        Item::Sub("Smart Pen", pen::menu()),
        Item::Sub("Type Tools", typetools::menu()),
        Item::Sub("Unit Converter", units::menu()),
        Item::Sep,
    ]
}
