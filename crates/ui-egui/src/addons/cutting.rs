//! Cutting Tools in the interface: its submenu (Object › Plug-ins › Cutting Tools). Stencil
//! Bridges… opens the generic command dialog (`ui.paramDialog`, `dialogs::command`) on the options
//! of `addon.cutting.bridges`. Weed Box and Lines… opens it on the options of `addon.cutting.weed`,
//! with a Preview, and runs the command on OK. A Spacing above 0 is sent instead of the Count.
//! Banner Hems and Grommets… opens it on the options of `addon.cutting.banner`, with a Preview.

use serde_json::json;

use crate::menus::Item;

/// Stencil Bridges' command.
pub const BRIDGES: &str = "addon.cutting.bridges";
/// Stencil Bridges › Direction: what the command's `direction` takes, by name.
pub const BRIDGE_DIRECTIONS: &[(&str, &str)] = &[("Vertical", "vertical"), ("Horizontal", "horizontal"), ("Shortest Crossing", "shortest")];
/// Weed Box and Lines' command.
pub const WEED: &str = "addon.cutting.weed";
/// Weed Box and Lines › Direction: what the command's `direction` takes, by name.
pub const WEED_DIRECTIONS: &[(&str, &str)] = &[("Horizontal", "horizontal"), ("Vertical", "vertical"), ("Both", "both")];
/// Banner Hems and Grommets' command.
pub const BANNER: &str = "addon.cutting.banner";

/// Object › Plug-ins › Cutting Tools.
pub fn menu() -> Vec<Item> {
    vec![
        Item::Cmd("Stencil Bridges…", BRIDGES, json!({"width": 2, "count": 1, "direction": "vertical", "keepOriginal": true})),
        Item::Cmd(
            "Weed Box and Lines…",
            WEED,
            json!({"margin": 9, "box": true, "direction": "both", "count": 1, "spacing": 0, "gap": 0, "weight": 0.3, "swatch": "[Registration]"}),
        ),
        Item::Cmd(
            "Banner Hems and Grommets…",
            BANNER,
            json!({
                "hem": 72, "top": true, "right": true, "bottom": true, "left": true, "grommets": true, "spacing": 1728, "grommetInset": 36,
                "diameter": 27, "weight": 0.3, "swatch": "[Registration]"
            }),
        ),
    ]
}

#[cfg(test)]
mod tests;
