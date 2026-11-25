use super::context::context_items;
use super::labels::{display_label, dynamic_label, effect_item_label, item_shortcut, spelling_item_label, unlock_item_label};
use super::slots::{recent_readable, recent_slot};
use super::state::{shortcut_of, shown_state};
use super::tree::{Listed, menu_tree_named};
use super::{Node, Serialize, TODO, Value, VectorcraftApp};

/// The label of the menu item that runs `id` with `p` and reads `label` in English, as the menus
/// draw it in the UI language: an effect's own name (Pathfinder › Add), an object's name, a
/// spelling suggestion, or [`display_label`].
pub fn shown_label(app: &VectorcraftApp, id: &str, p: &Value, label: &str) -> String {
    effect_item_label(id, p)
        .or_else(|| unlock_item_label(app, id, p, crate::i18n::current()))
        .or_else(|| spelling_item_label(id, p))
        .unwrap_or_else(|| display_label(app, id, label))
}

#[derive(Serialize)]
pub struct MenuEntry {
    pub path: Vec<String>,
    pub label: String,
    pub command: Option<String>,
    pub params: Value,
    pub enabled: bool,
    pub shortcut: String,
    /// `path` as the menus draw it in the UI language: a plug-in's submenu and a Scripts folder
    /// keep their names.
    #[serde(skip)]
    pub shown_path: Vec<String>,
}

/// Flattened menu for `ui.menu.list`.
pub fn menu_entries(app: &VectorcraftApp) -> Vec<MenuEntry> {
    let mut out = vec![];
    for (title, items) in menu_tree_named(app.session.prefs.font_names_in_english, &Listed::of(app)) {
        flatten(app, vec![title.to_string()], vec![crate::i18n::t(title).to_string()], &items, &mut out);
    }
    out
}

/// Flattened canvas context menu for `ui.contextMenu.list` (paths are its submenus).
pub fn context_entries(app: &VectorcraftApp) -> Vec<MenuEntry> {
    let mut out = vec![];
    flatten(app, vec![], vec![], &context_items(app), &mut out);
    out
}

/// A menu item that the Actions panel's Insert Menu Item… adds to an action as a step
/// ([`insertable_items`]).
#[derive(Clone, Debug, PartialEq)]
pub struct InsertableItem {
    /// The menu title and the submenus that hold the item, as the tree names them in English.
    pub path: Vec<String>,
    /// The item's label in the tree, in English.
    pub label: String,
    pub command: String,
    /// The item's params; `{}` for an item that has none.
    pub params: Value,
}

/// The menus whose items are not added to actions: the application menu, File (documents and
/// files), Window (panels and workspaces) and Help.
const NOT_INSERTED_MENUS: [&str; 4] = ["VectorCraft", "File", "Window", "Help"];

/// Commands whose menu items do more than run the command, which action playback does not do:
/// Object › Image Trace › Make and Make and Expand trace off the UI thread with progress and Cancel
/// ([`crate::trace_runs::start`]), Object › Pattern › Make and Edit Pattern open the Pattern
/// Options panel, and Edit › Find Next goes on from the last Find and Replace search or opens its
/// dialog ([`crate::dialogs::find_replace::find_next`]).
const MENU_ONLY_COMMANDS: [&str; 5] = ["imageTrace.make", "imageTrace.makeAndExpand", "object.pattern.make", "object.pattern.edit", "edit.findNext"];

/// The View menu commands that the app runs itself, not the engine, and that can be steps of an
/// action ([`plays_in_app`]): Outline, Pixel Preview, Trim View, Presentation Mode and Screen Mode,
/// the zoom, fit and rotate view items, and the items that show or hide or turn snapping on or off.
/// The saved-view slots are not among them.
const UI_STEP_COMMANDS: [&str; 30] = [
    "view.outline",
    "view.pixelPreview",
    "view.trimView",
    "view.presentation",
    "view.screenMode",
    "view.zoomIn",
    "view.zoomOut",
    "view.fitArtboard",
    "view.fitAll",
    "view.actualSize",
    "view.rotate",
    "view.rotateToSelection",
    "view.rotateReset",
    "view.edges",
    "view.artboards",
    "view.template",
    "view.rulers",
    "view.videoRulers",
    "view.boundingBox",
    "view.textThreads",
    "view.gradientAnnotator",
    "view.cornerWidget",
    "view.livePaintGaps",
    "view.guides",
    "view.smartGuides",
    "view.grid",
    "view.snapToGrid",
    "view.snapToPixel",
    "view.snapToPoint",
    "view.snapToGlyph",
];

/// Whether a step that runs command `id` is a View menu command that the app runs itself
/// ([`UI_STEP_COMMANDS`]). Action playback runs such a step on its own, as its menu item runs it,
/// and not inside `command.batch`.
pub fn plays_in_app(id: &str) -> bool {
    UI_STEP_COMMANDS.contains(&id)
}

/// Whether the item labeled `label` in menu `title`, which runs command `id` with `p`, can be a step
/// of an action. It can when it runs an engine command, which action playback runs as the menu
/// does, or is a View menu item that the app runs itself ([`plays_in_app`]); its label does not end
/// with "…", the mark of an item that asks for values first; it is not in a menu of
/// [`NOT_INSERTED_MENUS`]; and it is not Undo, Redo, a Paste, an item that stands for a saved
/// selection, a saved view or a recent font, a plug-in's item, an action's own command or another
/// item of [`MENU_ONLY_COMMANDS`]. An Object › Align item can be a step: inserting it adds the Align
/// To choice of that moment to its params, as clicking it does
/// ([`crate::panels::align::with_align_to`]).
pub fn insertable(title: &str, label: &str, id: &str, p: &Value) -> bool {
    use super::slots::{RECENT_FONT_IDS, SAVED_SELECTION_IDS, SAVED_VIEW_IDS};
    (vectorcraft_engine::find_command(id).is_some() || (title == "View" && plays_in_app(id)))
        && !label.ends_with('…')
        && !NOT_INSERTED_MENUS.contains(&title)
        && !matches!(id, "edit.undo" | "edit.redo" | "command.batch")
        && !MENU_ONLY_COMMANDS.contains(&id)
        && !id.starts_with("edit.paste")
        && !id.starts_with("prefs.")
        && !id.starts_with("action.")
        && ![&RECENT_FONT_IDS[..], &SAVED_SELECTION_IDS[..], &SAVED_VIEW_IDS[..]].iter().any(|ids| ids.contains(&id))
        && !super::labels::lists_plugin(id, p)
}

/// Every menu item that can be a step of an action ([`insertable`]), in menu order. An item that
/// shows in two places is listed in both. `english_names` and `listed` are as [`menu_tree_named`]
/// takes them.
pub fn insertable_items(english_names: bool, listed: &Listed) -> Vec<InsertableItem> {
    fn walk(title: &'static str, path: &mut Vec<String>, nodes: &[Node], out: &mut Vec<InsertableItem>) {
        for node in nodes {
            match node {
                Node::Item(it) if insertable(title, &it.label, &it.command.id, &it.command.params) => out.push(InsertableItem {
                    path: path.clone(),
                    label: it.label.clone(),
                    command: it.command.id.clone(),
                    params: if it.command.params.is_null() { serde_json::json!({}) } else { it.command.params.clone() },
                }),
                // A submenu with a role is a piece of the Mac application menu, not a menu of the tree.
                Node::Submenu { label, children, role: None, .. } => {
                    path.push(label.clone());
                    walk(title, path, children, out);
                    path.pop();
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    for (title, nodes) in menu_tree_named(english_names, listed) {
        walk(title, &mut vec![title.to_string()], &nodes, &mut out);
    }
    out
}

pub(super) fn flatten(app: &VectorcraftApp, path: Vec<String>, shown: Vec<String>, nodes: &[Node], out: &mut Vec<MenuEntry>) {
    for node in nodes {
        match node {
            Node::Item(it) if it.command.id == TODO => out.push(MenuEntry {
                path: path.clone(),
                label: it.label.clone(),
                command: None,
                params: Value::Null,
                enabled: false,
                shortcut: String::new(),
                shown_path: shown.clone(),
            }),
            Node::Item(it) => {
                let (l, id, p) = (it.label.as_str(), it.command.id.as_str(), &it.command.params);
                // A listed script or saved proof setup outside an automation read root is left out.
                if matches!(id, crate::scripts_menu::COMMAND | crate::dialogs::proof_setup::LOAD) && !crate::scripts_menu::readable(p) {
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
                    shortcut: item_shortcut(app, id, p).or_else(|| shortcut_of(app, id)).map(|c| c.to_string()).unwrap_or_default(),
                    shown_path: shown.clone(),
                })
            }
            // A submenu with a role is a piece of the Mac application menu, not a menu of the tree.
            Node::Submenu { label, children, role: None, .. } => {
                let mut p = path.clone();
                p.push(label.clone());
                // The menus draw a plug-in's submenu and a Scripts folder by their own names.
                let mut s = shown.clone();
                let own = super::labels::holds_plugin_commands(children) || crate::scripts_menu::is_folder(children);
                s.push(if own { label.clone() } else { crate::i18n::tr(crate::i18n::current(), label).to_string() });
                flatten(app, p, s, children, out);
            }
            _ => {}
        }
    }
}
