//! Image Tools in the interface: its submenu (Object › Plug-ins › Image Tools). Remove Background's
//! menu item opens the generic command dialog (`ui.paramDialog`) on its options, with a Preview
//! (`dialogs::command`). Color Mask's opens the same dialog on the selected image's mask settings
//! (the command's report, see `menus::invoke`), or the defaults for an image without one.

use serde_json::json;

use crate::menus::Item;

/// Remove Background's command.
pub const REMOVE_BACKGROUND: &str = "addon.image.removeBackground";
/// Color Mask's command.
pub const COLOR_MASK: &str = "addon.image.colorMask";
/// Remove Color Mask's command.
pub const REMOVE_COLOR_MASK: &str = "addon.image.removeColorMask";
/// Apply Color Mask's command.
pub const APPLY_COLOR_MASK: &str = "addon.image.applyColorMask";
/// Remove Background and Color Mask › Background: what the commands' `background` takes, by name.
pub const BACKGROUNDS: &[(&str, &str)] = &[("Edge Color", "edges"), ("Corner Color", "corners"), ("Picked Colors", "colors")];
/// Remove Background and Color Mask › Match: what the commands' `match` takes, by name.
pub const MATCHES: &[(&str, &str)] = &[("Distance", "distance"), ("Each Channel", "channels")];

/// Object › Plug-ins › Image Tools.
pub fn menu() -> Vec<Item> {
    vec![
        Item::Cmd(
            "Remove Background…",
            REMOVE_BACKGROUND,
            json!({"background": "edges", "colors": "#ffffff", "contiguous": true, "match": "distance", "tolerance": 10, "softness": 10}),
        ),
        Item::Cmd("Color Mask…", COLOR_MASK, json!({})),
        Item::Cmd("Remove Color Mask", REMOVE_COLOR_MASK, json!({})),
        Item::Cmd("Apply Color Mask", APPLY_COLOR_MASK, json!({})),
    ]
}

#[cfg(test)]
mod tests;
