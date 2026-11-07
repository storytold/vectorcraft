//! The Contour add-on in the interface: its submenu (Object › Plug-ins › Contour), which selects the
//! Contour tool. The Contour bar (`canvas/contour_bar.rs`) shows over the selection while the tool is
//! active. Everything acts through `addon.contour.set` and Expand Appearance.

use serde_json::json;
use vectorcraft_tools::addons::contour;

use crate::menus::Item;

pub fn menu() -> Vec<Item> {
    vec![Item::Cmd("Contour Tool", "tool.select", json!({"tool": contour::ID}))]
}
