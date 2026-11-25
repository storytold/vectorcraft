//! The menu tree (the full menu tree) and UI-level commands.
//!
//! Items bound to a command id run through [`VectorcraftApp::run`]. Items not implemented yet are
//! listed (disabled, with their shortcut) so the full surface is visible and discoverable; the
//! parity tracker drives them to "done".

use serde::Serialize;
use serde_json::{Value, json};

use vectorcraft_engine::cmd::fileio::SaveMode;

use crate::VectorcraftApp;
use crate::io;
use crate::panels::character::Face;
use crate::state::next_zoom;
use crate::theme::Brightness;

use craft::keymap::CommandRef;

pub(crate) use craft::menu::Node;
pub(crate) use craft::menu::Node::Separator as Sep;

/// The command id of an item that is not built yet; its state is always disabled.
pub(crate) const TODO: &str = "todo";

/// An item that runs command `id` with `params`, labelled `label` (the English source label for
/// tree rows, the shown name for rows that list names).
pub(crate) fn named(label: impl Into<String>, id: &str, params: Value) -> Node {
    Node::Item(craft::menu::Item::new(CommandRef::with_params(id, params), label))
}
pub(crate) fn c(label: &'static str, id: &'static str) -> Node {
    named(label, id, Value::Null)
}
pub(crate) fn cp(label: &'static str, id: &'static str, params: Value) -> Node {
    named(label, id, params)
}
/// An item not implemented yet; the model disables it and gives it its tooltip.
pub(crate) fn todo(label: &'static str) -> Node {
    named(label, TODO, json!({ "label": label }))
}
pub(crate) fn sub(label: &'static str, children: Vec<Node>) -> Node {
    Node::submenu(label, children)
}
/// A section caption (a disabled label, e.g. "Vector Effects").
pub(crate) fn heading(label: &'static str) -> Node {
    Node::heading(label)
}

mod context;
mod entries;
mod invoke;
mod labels;
pub(crate) mod model;
pub mod native;
mod run;
pub(crate) use run::TRIM_VIEW_HIDES;
mod slots;
mod state;
mod submenus;
mod tree;
mod ui_commands;

pub use context::context_items;
#[cfg(test)]
use entries::flatten;
pub use entries::{InsertableItem, MenuEntry, context_entries, insertable, insertable_items, menu_entries, plays_in_app, shown_label};
pub(crate) use invoke::graph_dialog;
pub use invoke::{click_target, invoke};
pub(crate) use labels::lists_plugin;
pub(crate) use labels::shows_a_name;
pub use labels::{CONTEXT_LABELS, DYNAMIC_LABELS, display_label, dynamic_label, item_shortcut, menu_strings};
pub(crate) use model::{MAC_LABELS, home_key, home_showing};
pub use run::{check_params, run_ui_command};
pub(crate) use run::{normalize_panel, opt_bool, opt_index, opt_number};
#[cfg(test)]
use slots::RECENT_FONT_IDS;
pub(crate) use state::why_off;
pub use state::{checked, enabled, plugin_revision, shortcut_of, shown_state};
pub(crate) use submenus::PATH_EFFECTS;
pub use tree::{Listed, menu_tree, menu_tree_named};
pub use ui_commands::UI_COMMANDS;

#[cfg(test)]
mod tests;
