//! The Contour add-on in the interface: its submenu (Object › Plug-ins › Contour), which selects the
//! Contour tool. The Contour bar (`canvas/contour_bar.rs`) shows over the selection while the tool is
//! active. Everything acts through `addon.contour.set` and Expand Appearance.

use craft::menu::Node;
use serde_json::json;
use vectorcraft_tools::addons::contour;

use crate::menus::cp;

pub fn menu() -> Vec<Node> {
    vec![cp("Contour Tool", "tool.select", json!({"tool": contour::ID}))]
}
