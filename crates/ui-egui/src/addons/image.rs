//! Image Tools in the interface: its submenu (Object › Plug-ins › Image Tools). Remove Background's
//! menu item opens the generic command dialog (`ui.paramDialog`) on its options, with a Preview
//! (`dialogs::command`).

use serde_json::json;

use crate::menus::Item;

/// Remove Background's command.
pub const REMOVE_BACKGROUND: &str = "addon.image.removeBackground";
/// Remove Background › Background: what the command's `background` takes, by name.
pub const BACKGROUNDS: &[(&str, &str)] = &[("Edge Color", "edges"), ("Corner Color", "corners"), ("Picked Colors", "colors")];
/// Remove Background › Match: what the command's `match` takes, by name.
pub const MATCHES: &[(&str, &str)] = &[("Distance", "distance"), ("Each Channel", "channels")];

/// Object › Plug-ins › Image Tools.
pub fn menu() -> Vec<Item> {
    vec![Item::Cmd(
        "Remove Background…",
        REMOVE_BACKGROUND,
        json!({"background": "edges", "colors": "#ffffff", "contiguous": true, "match": "distance", "tolerance": 10, "softness": 10}),
    )]
}

#[cfg(test)]
mod tests;
