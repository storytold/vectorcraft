use super::context::context_items;
use super::invoke::click_target;
use super::labels::{display_label, dynamic_label, effect_item_label, item_shortcut, unlock_item_label};
use super::state::{shortcut_of, shown_state};
use super::tree::menu_tree_named;
use super::*;

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

fn flatten(app: &VectorcraftApp, path: Vec<String>, items: &[Item], out: &mut Vec<MenuEntry>) {
    for it in items {
        match it {
            Item::Cmd(l, id, p) => {
                let Some((enabled, _)) = shown_state(app, id, p) else { continue };
                out.push(MenuEntry {
                    path: path.clone(),
                    label: unlock_item_label(app, id, p, crate::i18n::Lang::EN).unwrap_or_else(|| dynamic_label(app, id, l)),
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
