//! Portable desktop snapshot over the existing menu tree. The Linux host exports it without
//! hiding the local menu. Native clicks retain the same dialog and focused-field routing.

use craft_desktop::{Action, Item, Menu};
use serde_json::{Value, json};

use crate::{VectorcraftApp, menus, native_menu};

/// Native menu work has the same last-resort recovery boundary as a local UI frame.
fn guarded<T>(app: &mut VectorcraftApp, work: impl FnOnce(&mut VectorcraftApp) -> T) -> Option<T> {
    match vectorcraft_engine::guard::catch_panic(|| work(app)) {
        Ok(value) => Some(value),
        Err(msg) => {
            app.picks.recover();
            app.status(format!("Internal error: {msg} (please report this bug)"));
            None
        }
    }
}

/// Build a changed snapshot inside the recovery boundary. The host only advances its
/// cached key after success, so failed projection is retried on a subsequent frame.
pub fn snapshot_if_changed(app: &mut VectorcraftApp, ctx: &egui::Context, previous: Option<u64>) -> Option<(u64, Menu)> {
    guarded(app, |app| {
        let state = state_key(app, ctx);
        (previous != Some(state)).then(|| (state, snapshot(app)))
    })
    .flatten()
}

pub fn state_key(app: &VectorcraftApp, ctx: &egui::Context) -> u64 {
    use std::hash::{Hash, Hasher};
    let input = ctx
        .input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: false, .. })));
    let mut hash = std::hash::DefaultHasher::new();
    native_menu::state_hash(app, input.then_some(app.host_iteration)).hash(&mut hash);
    app.ui.dialog.is_some().hash(&mut hash);
    hash.finish()
}

pub fn snapshot(app: &VectorcraftApp) -> Menu {
    let bar = native_menu::from_tree(app, &menus::menu_tree_named(app.session.prefs.font_names_in_english));
    let blocked = app.file_dialog_open() || app.ui.dialog.is_some();
    fn level(app: &VectorcraftApp, nodes: &[native_menu::Node], blocked: bool) -> Vec<Item> {
        nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                Some(match node {
                    native_menu::Node::Item(row) => {
                        let action = row.command.map(|command| {
                            // Slot commands (Recent Files, saved selections, views, fonts, libraries)
                            // resolve from current state. Keep their displayed identity too, so a queued
                            // click is not silently redirected after a slot changes. Recent paths need
                            // more than their basename: two files can have the same displayed name.
                            let recent = command
                                .strip_prefix("file.openRecent")
                                .and_then(|n| n.parse::<usize>().ok())
                                .and_then(|n| n.checked_sub(1))
                                .and_then(|n| app.ui.recent_files.get(n));
                            Action {
                                command: command.into(),
                                params: json!({
                                    "source": row.source, "arguments": row.params, "identity": row.label, "recent": recent,
                                }),
                            }
                        });
                        Item {
                            key: format!("{}:{}:{}", row.command.unwrap_or("unavailable"), row.source, row.params),
                            label: row.label.clone(),
                            enabled: row.enabled && !blocked,
                            checked: row.checked,
                            shortcut: row.shortcut.unwrap_or_default().replace("Cmd", "Ctrl"),
                            action,
                            ..Default::default()
                        }
                    }
                    native_menu::Node::Separator => Item { key: format!("separator-{index}"), separator: true, ..Default::default() },
                    native_menu::Node::Header(label) => Item { key: format!("header-{index}"), label: label.clone(), ..Default::default() },
                    native_menu::Node::Submenu { label, children } => Item {
                        key: format!("submenu-{index}"),
                        label: label.clone(),
                        enabled: !blocked,
                        children: level(app, children, blocked),
                        ..Default::default()
                    },
                    native_menu::Node::Standard(_) => return None,
                })
            })
            .collect()
    }
    Menu {
        document: app.session.active().map_or(0, |doc| doc.uid),
        items: bar
            .menus
            .iter()
            .enumerate()
            .map(|(index, menu)| Item {
                key: format!("menu-{index}"),
                label: menu.title.clone(),
                enabled: !blocked,
                children: level(app, &menu.children, blocked),
                ..Default::default()
            })
            .collect(),
    }
}

fn contains_enabled(items: &[Item], action: &Action) -> bool {
    items.iter().any(|item| item.enabled && (item.action.as_ref() == Some(action) || contains_enabled(&item.children, action)))
}

/// The action carries menu presentation context, not engine parameters directly. Revalidate
/// the full envelope before extracting its original command arguments and interactive target.
pub fn activate(app: &mut VectorcraftApp, ctx: &egui::Context, document: u64, action: &Action) {
    if guarded(app, |app| activate_inner(app, ctx, document, action)).is_none() {
        ctx.request_repaint();
    }
}

fn activate_inner(app: &mut VectorcraftApp, ctx: &egui::Context, document: u64, action: &Action) {
    let current = snapshot(app);
    if current.document != document || !contains_enabled(&current.items, action) {
        return;
    }
    let Some(source) = action.params.get("source").and_then(Value::as_str) else { return };
    let Some(arguments) = action.params.get("arguments") else { return };
    let (id, params) = menus::click_target(source, &action.command, arguments);
    menus::invoke_from_system_menu(app, ctx, &id, params);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default())
    }

    fn find<'a>(items: &'a [Item], command: &str) -> Option<&'a Item> {
        items.iter().find_map(
            |item| {
                if item.action.as_ref().is_some_and(|a| a.command == command) { Some(item) } else { find(&item.children, command) }
            },
        )
    }

    #[test]
    fn desktop_menu_new_opens_interactive_dialog_and_keeps_local_menus() {
        let mut app = app();
        let menu = snapshot(&app);
        let action = find(&menu.items, "file.newDialog").unwrap().action.as_ref().unwrap();
        activate(&mut app, &egui::Context::default(), menu.document, action);
        assert!(app.ui.dialog.is_some());
        assert!(!find(&snapshot(&app).items, "file.newDialog").unwrap().enabled);
        assert!(app.services.native_menu.is_none());
    }

    #[test]
    fn desktop_menu_rejects_stale_recent_file_slot_and_document() {
        let mut app = app();
        app.ui.recent_files = vec!["/tmp/a/same.svg".into()];
        let menu = snapshot(&app);
        let recent = find(&menu.items, "file.openRecent1").unwrap().action.clone().unwrap();
        app.ui.recent_files = vec!["/tmp/b/same.svg".into()];
        assert!(!contains_enabled(&snapshot(&app).items, &recent));
        let new = find(&menu.items, "file.newDialog").unwrap().action.clone().unwrap();
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        activate(&mut app, &egui::Context::default(), menu.document, &new);
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn desktop_menu_matches_local_command_states_and_shortcuts() {
        let mut app = app();
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        for menu in native_menu::from_tree(&app, &menus::menu_tree()).menus {
            for row in (native_menu::MenuBar { menus: vec![menu] }).items() {
                if let Some(command) = row.command {
                    let snapshot = snapshot(&app);
                    let found = find(&snapshot.items, command);
                    assert!(found.is_some(), "missing local command {command}");
                }
            }
        }
        let menu = snapshot(&app);
        let undo = find(&menu.items, "edit.undo").unwrap();
        assert!(!undo.enabled);
        assert!(!undo.shortcut.contains("Cmd"));
    }

    #[test]
    fn desktop_menu_checkmarks_follow_local_view_state() {
        let mut app = app();
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let before = snapshot(&app);
        let rulers = find(&before.items, "view.rulers").unwrap();
        let checked = rulers.checked.unwrap();
        activate(&mut app, &egui::Context::default(), before.document, rulers.action.as_ref().unwrap());
        assert_eq!(find(&snapshot(&app).items, "view.rulers").unwrap().checked, Some(!checked));
    }

    #[test]
    fn desktop_menu_panic_recovery_preserves_document_and_allows_next_action() {
        let mut app = app();
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let document = app.session.active().unwrap().uid;
        assert!(guarded::<()>(&mut app, |_| panic!("synthetic native menu failure")).is_none());
        assert!(app.ui.status.contains("Internal error: synthetic native menu failure"));
        assert_eq!(app.session.active().unwrap().uid, document);
        let ctx = egui::Context::default();
        let (state, menu) = snapshot_if_changed(&mut app, &ctx, None).unwrap();
        assert!(snapshot_if_changed(&mut app, &ctx, Some(state)).is_none());
        let rulers = find(&menu.items, "view.rulers").unwrap();
        activate(&mut app, &ctx, document, rulers.action.as_ref().unwrap());
        let (_, refreshed) = snapshot_if_changed(&mut app, &ctx, Some(state)).unwrap();
        assert_eq!(find(&refreshed.items, "view.rulers").unwrap().checked, rulers.checked.map(|checked| !checked));
    }

    #[test]
    fn desktop_menu_automatic_palette_change_invalidates_cached_snapshot() {
        let mut app = app();
        app.session.prefs.ui_brightness = "system".into();
        let ctx = egui::Context::default();
        app.ui.brightness = crate::theme::Brightness::Light;
        let (light, _) = snapshot_if_changed(&mut app, &ctx, None).unwrap();
        app.ui.brightness = crate::theme::Brightness::MediumDark;
        assert!(snapshot_if_changed(&mut app, &ctx, Some(light)).is_some());
    }
}
