//! Cutting Tools in the interface: its submenu (Object › Plug-ins › Cutting Tools). Stencil
//! Bridges… opens the generic command dialog (`ui.paramDialog`, `dialogs::command`) on the options
//! of `addon.cutting.bridges`.

use serde_json::json;

use crate::menus::Item;

/// Stencil Bridges' command.
pub const BRIDGES: &str = "addon.cutting.bridges";
/// Stencil Bridges › Direction: what the command's `direction` takes, by name.
pub const BRIDGE_DIRECTIONS: &[(&str, &str)] = &[("Vertical", "vertical"), ("Horizontal", "horizontal"), ("Shortest Crossing", "shortest")];

/// Object › Plug-ins › Cutting Tools.
pub fn menu() -> Vec<Item> {
    vec![Item::Cmd("Stencil Bridges…", BRIDGES, json!({"width": 2, "count": 1, "direction": "vertical", "keepOriginal": true}))]
}

#[cfg(test)]
mod tests;
