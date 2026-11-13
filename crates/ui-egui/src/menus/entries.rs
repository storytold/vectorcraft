use super::context::context_items;
use super::invoke::click_target;
use super::labels::{display_label, dynamic_label, effect_item_label, item_shortcut, spelling_item_label, unlock_item_label};
use super::slots::{recent_readable, recent_slot};
use super::state::{shortcut_of, shown_state};
use super::tree::menu_tree_named;
use super::{Entry, Item, Serialize, Value, VectorcraftApp};

/// A menu item as it shows now ([`entry`]).
pub struct Row<'a> {
    /// The command and its params; `None` for an item not implemented yet (always disabled).
    pub command: Option<(&'static str, &'a Value)>,
    /// The tree's own English label, which [`click_target`] reads.
    pub source: &'static str,
    /// The label in the UI language, as it reads now (Undo *Move*, Show/Hide…, a recent file).
    pub label: std::borrow::Cow<'static, str>,
    /// The shortcut as the registry writes it (`Cmd+Shift+Z`), the user's overrides applied.
    pub shortcut: Option<&'static str>,
    pub enabled: bool,
    /// `Some` for an item that is on or off.
    pub checked: Option<bool>,
}

impl Row<'_> {
    /// What choosing it runs ([`click_target`]); `None` for an item not implemented yet.
    pub fn target(&self) -> Option<(String, Value)> {
        self.command.map(|(id, p)| click_target(self.source, id, p))
    }
}

/// Item `it` as the menus show it now, or `None` while they leave it out ([`shown_state`]).
pub fn entry<'a>(app: &VectorcraftApp, it: &'a Item) -> Option<Entry<'a>> {
    use crate::i18n::t;
    Some(match it {
        Item::Sep => Entry::Sep,
        Item::Header(h) => Entry::Header(t(h)),
        // A plug-in's submenu shows its name as it is, as its items do, and so does a subfolder of
        // the Scripts folder.
        Item::Sub(label, children) if super::labels::holds_plugin_commands(children) || crate::scripts_menu::is_folder(children) => {
            Entry::Sub(label, children)
        }
        Item::Sub(label, children) => Entry::Sub(t(label), children),
        Item::Todo(label, sc) => Entry::Item(Row {
            command: None,
            source: label,
            label: t(label).into(),
            shortcut: Some(*sc).filter(|s| !s.is_empty()),
            enabled: false,
            checked: None,
        }),
        Item::Cmd(label, id, p) => {
            let (enabled, checked) = shown_state(app, id, p)?;
            Entry::Item(Row {
                command: Some((id, p)),
                source: label,
                label: effect_item_label(id, p)
                    .or_else(|| unlock_item_label(app, id, p, crate::i18n::current()))
                    .or_else(|| spelling_item_label(id, p))
                    .unwrap_or_else(|| display_label(app, id, label))
                    .into(),
                shortcut: item_shortcut(id, p),
                enabled,
                checked,
            })
        }
    })
}

/// `Cmd+Shift+]` → `⇧⌘]` on macOS, `Ctrl+Shift+]` elsewhere.
pub fn pretty_shortcut(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        let mut mods = String::new();
        let parts: Vec<&str> = s.split('+').collect();
        let (key, ms): (&str, &[&str]) = if s.ends_with("++") {
            ("+", parts.get(..parts.len().saturating_sub(2)).unwrap_or_default())
        } else {
            parts.split_last().map_or(("", &[][..]), |(k, rest)| (*k, rest))
        };
        for m in ["Ctrl", "Alt", "Shift", "Cmd"] {
            if ms.contains(&m) {
                mods.push_str(match m {
                    "Ctrl" => "⌃",
                    "Alt" => "⌥",
                    "Shift" => "⇧",
                    _ => "⌘",
                });
            }
        }
        format!("{mods}{key}")
    } else {
        s.replace("Cmd", "Ctrl")
    }
}

/// [`pretty_shortcut`] with the backtick key shown as the character it types on the layout in use
/// (`"` on Turkish Q), as Edit › Keyboard Shortcuts shows it.
pub fn shown_shortcut(s: &str) -> String {
    pretty_shortcut(&crate::shortcut_editor::as_typed(s))
}

#[derive(Serialize)]
pub struct MenuEntry {
    pub path: Vec<String>,
    pub label: String,
    pub command: Option<String>,
    pub params: Value,
    pub enabled: bool,
    pub shortcut: String,
}

/// Flattened menu for `ui.menu.list`.
pub fn menu_entries(app: &VectorcraftApp) -> Vec<MenuEntry> {
    let mut out = vec![];
    for (title, items) in menu_tree_named(app.session.prefs.font_names_in_english) {
        flatten(app, vec![title.to_string()], &items, &mut out);
    }
    out
}

/// Flattened canvas context menu for `ui.contextMenu.list` (paths are its submenus).
pub fn context_entries(app: &VectorcraftApp) -> Vec<MenuEntry> {
    let mut out = vec![];
    flatten(app, vec![], &context_items(app), &mut out);
    out
}

/// A menu item that the Actions panel's Insert Menu Item… adds to an action as a step
/// ([`insertable_items`]).
#[derive(Clone, Debug, PartialEq)]
pub struct InsertableItem {
    /// The menu title and the submenus that hold the item, as the tree names them in English.
    pub path: Vec<&'static str>,
    /// The item's label in the tree, in English.
    pub label: &'static str,
    pub command: &'static str,
    /// The item's params; `{}` for an item that has none.
    pub params: Value,
}

/// The menus whose items are not added to actions: the application menu, File (documents and
/// files), Window (panels and workspaces) and Help.
const NOT_INSERTED_MENUS: [&str; 4] = ["VectorCraft", "File", "Window", "Help"];

/// Commands whose menu items do more than run the command, which action playback does not do:
/// Object › Image Trace › Make and Make and Expand trace off the UI thread with progress and Cancel
/// ([`crate::trace_runs::start`]), and Object › Pattern › Make and Edit Pattern open the Pattern
/// Options panel.
const MENU_ONLY_COMMANDS: [&str; 4] = ["imageTrace.make", "imageTrace.makeAndExpand", "object.pattern.make", "object.pattern.edit"];

/// Whether the item labeled `label` in menu `title`, which runs command `id` with `p`, can be a step
/// of an action. It can when it runs an engine command, which action playback runs as the menu
/// does; its label does not end with "…", the mark of an item that asks for values first; it is not
/// in a menu of [`NOT_INSERTED_MENUS`]; and it is not Undo, Redo, a Paste, an item that stands for a
/// saved selection, a saved view or a recent font, a plug-in's item or an action's own command. Nor
/// is it Object › Align, whose menu items add the Align To choice and the selection's key to their
/// params when clicked ([`crate::panels::align::with_align_to`]), which playback does not, or another
/// item of [`MENU_ONLY_COMMANDS`].
pub fn insertable(title: &str, label: &str, id: &str, p: &Value) -> bool {
    use super::slots::{RECENT_FONT_IDS, SAVED_SELECTION_IDS, SAVED_VIEW_IDS};
    vectorcraft_engine::find_command(id).is_some()
        && !label.ends_with('…')
        && !NOT_INSERTED_MENUS.contains(&title)
        && !matches!(id, "edit.undo" | "edit.redo" | "command.batch" | "object.align")
        && !MENU_ONLY_COMMANDS.contains(&id)
        && !id.starts_with("edit.paste")
        && !id.starts_with("prefs.")
        && !id.starts_with("action.")
        && ![&RECENT_FONT_IDS[..], &SAVED_SELECTION_IDS[..], &SAVED_VIEW_IDS[..]].iter().any(|ids| ids.contains(&id))
        && !super::labels::lists_plugin(id, p)
}

/// Every menu item that can be a step of an action ([`insertable`]), in menu order. An item that
/// shows in two places is listed in both. `english_names` is as [`menu_tree_named`] takes it.
pub fn insertable_items(english_names: bool) -> Vec<InsertableItem> {
    fn walk(title: &'static str, path: &mut Vec<&'static str>, items: &[Item], out: &mut Vec<InsertableItem>) {
        for it in items {
            match it {
                Item::Cmd(label, id, p) if insertable(title, label, id, p) => out.push(InsertableItem {
                    path: path.clone(),
                    label,
                    command: id,
                    params: if p.is_null() { serde_json::json!({}) } else { p.clone() },
                }),
                Item::Sub(label, children) => {
                    path.push(label);
                    walk(title, path, children, out);
                    path.pop();
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    for (title, items) in menu_tree_named(english_names) {
        walk(title, &mut vec![title], &items, &mut out);
    }
    out
}

fn flatten(app: &VectorcraftApp, path: Vec<String>, items: &[Item], out: &mut Vec<MenuEntry>) {
    for it in items {
        match it {
            Item::Cmd(l, id, p) => {
                if *id == crate::scripts_menu::COMMAND && !crate::scripts_menu::readable(p) {
                    continue;
                }
                // The other rows keep their ids: `file.openRecent3` still opens the third file.
                if id.starts_with("file.openRecent") && recent_slot(app, id).is_some_and(|path| !recent_readable(path)) {
                    continue;
                }
                let Some((enabled, _)) = shown_state(app, id, p) else { continue };
                out.push(MenuEntry {
                    path: path.clone(),
                    label: unlock_item_label(app, id, p, crate::i18n::Lang::EN)
                        .or_else(|| spelling_item_label(id, p))
                        .unwrap_or_else(|| dynamic_label(app, id, l)),
                    command: Some(id.to_string()),
                    params: p.clone(),
                    enabled,
                    shortcut: item_shortcut(id, p).or_else(|| shortcut_of(id)).unwrap_or("").to_string(),
                })
            }
            Item::Todo(l, sc) => out.push(MenuEntry {
                path: path.clone(),
                label: l.to_string(),
                command: None,
                params: Value::Null,
                enabled: false,
                shortcut: sc.to_string(),
            }),
            Item::Sub(l, ch) => {
                let mut p = path.clone();
                p.push(l.to_string());
                flatten(app, p, ch, out);
            }
            _ => {}
        }
    }
}
