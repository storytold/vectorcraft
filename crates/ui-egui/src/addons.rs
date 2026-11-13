//! The module defines VectorCraft's built-in add-on menus and panels. Object › Plug-ins lists their tools and commands.

pub mod arrange;
pub mod cleanup;
pub mod color;
pub mod compounds;
pub mod contour;
pub mod cutting;
pub mod document;
pub mod enclosing_lasso;
pub mod image;
pub mod pathtools;
pub mod pen;
pub mod select;
pub mod shapes;
pub mod typetools;

use crate::menus::Item;

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
        Item::Sub("Path Precision Tools", pathtools::menu()),
        Item::Sub("Selection Tools", select::menu()),
        Item::Sub("Smart Pen", pen::menu()),
        Item::Sub("Type Tools", typetools::menu()),
        Item::Sep,
    ]
}
