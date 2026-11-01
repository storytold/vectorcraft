//! VectorCraft's built-in add-ons in the interface: each has a panel and a submenu at the top of
//! Object › Plug-ins, and acts through its engine commands (`addon.<name>.*`).

pub mod arrange;
pub mod cleanup;
pub mod color;
pub mod document;
pub mod pathtools;
pub mod pen;

use crate::menus::Item;

/// The add-ons' submenus, then a separator: the head of Object › Plug-ins.
pub fn plugin_menu() -> Vec<Item> {
    vec![
        Item::Sub("Arrange Tools", arrange::menu()),
        Item::Sub("Artwork Cleanup", cleanup::menu()),
        Item::Sub("Color Effects", color::menu()),
        Item::Sub("Document Tools", document::menu()),
        Item::Sub("Path Precision Tools", pathtools::menu()),
        Item::Sub("Smart Pen", pen::menu()),
        Item::Sep,
    ]
}
