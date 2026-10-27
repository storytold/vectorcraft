//! The menu bar (Illustrator's menu tree) and UI-level commands.
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
use crate::theme::{Brightness, Tokens};
use crate::widgets;

#[derive(Clone, Debug)]
pub enum Item {
    /// (label, command id, params)
    Cmd(&'static str, &'static str, Value),
    /// Not implemented yet: (label, shortcut)
    Todo(&'static str, &'static str),
    Sub(&'static str, Vec<Item>),
    Sep,
    /// Section header (disabled label, e.g. "Vector Effects").
    Header(&'static str),
}

fn c(label: &'static str, id: &'static str) -> Item {
    Item::Cmd(label, id, Value::Null)
}
fn cp(label: &'static str, id: &'static str, p: Value) -> Item {
    Item::Cmd(label, id, p)
}
fn todo(label: &'static str) -> Item {
    Item::Todo(label, "")
}
fn todos(label: &'static str, sc: &'static str) -> Item {
    Item::Todo(label, sc)
}
fn sub(label: &'static str, items: Vec<Item>) -> Item {
    Item::Sub(label, items)
}
use Item::Sep;

mod bar;
mod context;
mod entries;
mod invoke;
mod labels;
mod run;
mod slots;
mod state;
mod submenus;
mod tree;
mod ui_commands;

pub use bar::menu_bar;
pub(crate) use bar::{home_key, home_showing};
#[cfg(test)]
use bar::{menu_body, pointer_reaches_title};
pub use context::{context_items, context_menu_body, ruler_menu_body, ruler_unit_items};
pub use entries::{MenuEntry, Row, context_entries, entry, menu_entries, pretty_shortcut};
pub(crate) use invoke::graph_dialog;
pub use invoke::{click_target, invoke, invoke_from_system_menu};
#[cfg(test)]
pub(crate) use labels::lists_plugin;
#[cfg(test)]
use labels::shows_a_name;
pub use labels::{CONTEXT_LABELS, DYNAMIC_LABELS, display_label, dynamic_label, item_shortcut, menu_strings};
pub use run::{check_params, run_ui_command};
pub(crate) use run::{normalize_panel, opt_bool};
#[cfg(test)]
use slots::RECENT_FONT_IDS;
pub use state::{checked, enabled, plugin_revision, shortcut_of, shown_state};
pub(crate) use submenus::PATH_EFFECTS;
pub use tree::{menu_tree, menu_tree_named};
pub use ui_commands::UI_COMMANDS;

/// One row of a menu as it shows now. The in-window menus draw these and the macOS menu bar
/// ([`crate::native_menu`]) is built from them, so the two can't drift.
pub enum Entry<'a> {
    Sep,
    /// A section header (a disabled label), in the UI language.
    Header(&'static str),
    /// A submenu: its label in the UI language, and its items.
    Sub(&'static str, &'a [Item]),
    Item(Row<'a>),
}

#[cfg(test)]
mod tests;
