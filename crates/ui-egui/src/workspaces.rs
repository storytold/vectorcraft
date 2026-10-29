//! Window → Workspace: saved panel/bar layouts (Essentials, Essentials Classic, Painting, …),
//! Reset, New Workspace…, Manage Workspaces…. A workspace is a snapshot of `UiState` layout
//! flags; UI brightness and everything else is untouched when switching.
//!
//! As in Illustrator, a workspace remembers the changes made in it: switching to another one keeps
//! the layout as it was left (`UiState::workspace_changes`, kept with the preferences), switching
//! back brings it back, and Reset <name> returns to the workspace as saved. New Workspace… with
//! the name of a user workspace saves the current layout as it.

use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::{Dialog, DockTab, FloatingFlyout, FloatingPanels, UiState};
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Workspace {
    pub name: String,
    pub control_bar: bool,
    pub rulers: bool,
    pub toolbar: bool,
    pub toolbar_double: bool,
    pub toolbar_advanced: bool,
    pub task_bar: bool,
    pub dock: bool,
    pub dock_tab: DockTab,
    /// The dock's tabbed group is collapsed to icons (built-in workspaces: expanded).
    pub dock_collapsed: bool,
    pub open_panel: Option<String>,
    pub status_bar: bool,
    /// Tool groups torn off the toolbar into floating strips (built-in workspaces: none).
    pub floating_flyouts: Vec<FloatingFlyout>,
    /// Floating panel groups in workspaces saved before the canvas joined `docking`; read only
    /// to convert them ([`crate::layout_migration`]).
    #[serde(skip_serializing)]
    pub floating_panels: Vec<FloatingPanels>,
    /// The workspace's own panel layout; none: the default layout from its flags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docking: Option<craft_ui::docking::Layout<String>>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub docking_hidden: std::collections::BTreeMap<String, craft_ui::docking::Location<String>>,
    /// Panels dropped on the icon column, in older workspaces (read only).
    #[serde(default, skip_serializing)]
    pub docking_icons: std::collections::BTreeSet<String>,
    /// The panels shown by dock groups collapsed to their tabs, in older workspaces (read only).
    #[serde(default, skip_serializing)]
    pub docking_collapsed: std::collections::BTreeSet<String>,
    /// Where the Tools panel floats (built-in workspaces: docked).
    pub toolbar_pos: Option<[f32; 2]>,
    /// Main dock column width in screen points while `docking` is none (#1753).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock_width: Option<f32>,
}

impl Default for Workspace {
    fn default() -> Self {
        let ui = UiState::default();
        capture(&ui, "Essentials")
    }
}

pub const ESSENTIALS: &str = "Essentials";
/// Touch workspace (Window → Workspace → Touch; Preferences › Devices › Enable Touch Workspace).
pub const TOUCH: &str = "Touch";

/// Built-in workspaces, in Illustrator's menu order (Essentials first).
pub fn builtins() -> Vec<Workspace> {
    let base = Workspace::default();
    let w = |name: &str, f: &dyn Fn(&mut Workspace)| {
        let mut x = Workspace { name: name.into(), ..base.clone() };
        f(&mut x);
        x
    };
    vec![
        w(ESSENTIALS, &|_| {}),
        w("Essentials Classic", &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
            x.task_bar = false;
            x.open_panel = Some("color".into());
        }),
        w("Automation", &|x| {
            x.control_bar = true;
            x.toolbar_advanced = true;
            x.dock_tab = DockTab::Layers;
            x.open_panel = Some("actions".into());
        }),
        w("Layout", &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
            x.dock_tab = DockTab::Layers;
            x.open_panel = Some("align".into());
        }),
        w("Painting", &|x| {
            x.control_bar = true;
            x.toolbar_advanced = true;
            x.toolbar_double = true;
            x.open_panel = Some("brushes".into());
        }),
        w("Printing and Proofing", &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
            x.open_panel = Some("info".into());
        }),
        w("Tracing", &|x| {
            x.control_bar = true;
            x.toolbar_advanced = true;
            x.dock_tab = DockTab::Layers;
            x.task_bar = false;
        }),
        w("Typography", &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
            x.open_panel = Some("character".into());
        }),
        w("Web", &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
            x.open_panel = Some("symbols".into());
        }),
        w(TOUCH, &|x| {
            x.control_bar = true;
            x.toolbar = true;
            x.toolbar_advanced = true;
            x.task_bar = true;
            x.open_panel = Some("align".into());
        }),
    ]
}

/// Preferences › Devices › Enable Touch Workspace (#394): on, a touch screen switches to the Touch
/// workspace; off, or when no touch screen is reported, the workspace before that switch is
/// restored if Touch was entered automatically.
pub fn sync_touch_workspace(app: &mut VectorcraftApp, touch_screen: bool) {
    let auto = app.session.prefs.touch_workspace && touch_screen;
    if auto {
        if app.ui.workspace != TOUCH {
            if app.ui.touch_workspace_restore.is_none() {
                app.ui.touch_workspace_restore = Some(app.ui.workspace.clone());
            }
            if let Some(w) = find(&app.ui, TOUCH) {
                apply(&mut app.ui, &w);
            }
        }
    } else if app.ui.touch_workspace_restore.is_some() && app.ui.workspace == TOUCH {
        let back = app.ui.touch_workspace_restore.take().filter(|n| find(&app.ui, n).is_some()).unwrap_or_else(|| ESSENTIALS.to_string());
        if let Some(w) = find(&app.ui, &back) {
            apply(&mut app.ui, &w);
        }
    } else if !auto {
        app.ui.touch_workspace_restore = None;
    }
}

pub fn is_builtin(name: &str) -> bool {
    builtins().iter().any(|w| w.name == name)
}

/// Snapshot the layout flags of `ui` as a workspace called `name`.
pub fn capture(ui: &UiState, name: &str) -> Workspace {
    Workspace {
        name: name.into(),
        control_bar: ui.control_bar,
        rulers: ui.view.rulers,
        toolbar: ui.toolbar,
        toolbar_double: ui.toolbar_double,
        toolbar_advanced: ui.toolbar_advanced,
        task_bar: ui.task_bar,
        dock: ui.dock,
        dock_tab: ui.dock_tab,
        dock_collapsed: ui.dock_collapsed,
        open_panel: ui.open_panel.clone(),
        status_bar: ui.status_bar,
        floating_flyouts: ui.floating_flyouts.clone(),
        floating_panels: Vec::new(),
        docking: ui.docking.clone(),
        docking_hidden: ui.docking_hidden.clone(),
        docking_icons: Default::default(),
        docking_collapsed: Default::default(),
        toolbar_pos: ui.toolbar_pos,
        dock_width: ui.dock_width,
    }
}

/// Apply a workspace's layout flags (brightness, prefs, shortcuts etc. untouched).
pub fn apply(ui: &mut UiState, w: &Workspace) {
    ui.control_bar = w.control_bar;
    ui.view.rulers = w.rulers;
    ui.toolbar = w.toolbar;
    ui.toolbar_double = w.toolbar_double;
    ui.toolbar_advanced = w.toolbar_advanced;
    ui.task_bar = w.task_bar;
    ui.dock = w.dock;
    ui.tab_hid = None;
    ui.dock_tab = w.dock_tab;
    ui.dock_collapsed = w.dock_collapsed;
    ui.open_panel = w.open_panel.clone();
    ui.status_bar = w.status_bar;
    ui.floating_flyouts = w.floating_flyouts.clone();
    ui.docking_generation = ui.docking_generation.wrapping_add(1);
    ui.docking = w.docking.clone().filter(crate::panel_docking::valid);
    ui.docking_hidden = w.docking_hidden.clone();
    ui.toolbar_pos = w.toolbar_pos;
    ui.dock_width = w.dock_width;
    crate::state::sanitize_toolbar_pos(&mut ui.toolbar_pos);
    ui.flyout = None;
    ui.workspace = w.name.clone();
}

/// Keep the layout of the current workspace as it is now, so switching back brings it back.
fn remember(ui: &mut UiState) {
    // Kept under the saved workspace's spelling, which `find_saved` matches whatever the case.
    let Some(saved) = find_saved(ui, &ui.workspace.clone()) else { return };
    let w = capture(ui, &saved.name);
    ui.workspace_changes.retain(|x| !x.name.eq_ignore_ascii_case(&saved.name));
    ui.workspace_changes.push(w);
    // Over the cap, the oldest go: the layout just kept is the newest.
    let over = ui.workspace_changes.len().saturating_sub(64);
    ui.workspace_changes.drain(..over);
}

/// Switch to workspace `name` as it was left (its changes since it was saved), else as saved.
pub fn switch(ui: &mut UiState, name: &str) -> Result<String, String> {
    let saved = find_saved(ui, name).ok_or_else(|| format!("unknown workspace (one of {})", names(ui).join(", ")))?;
    remember(ui);
    let w = ui.workspace_changes.iter().find(|w| w.name.eq_ignore_ascii_case(&saved.name)).cloned().unwrap_or(saved);
    apply(ui, &w);
    Ok(w.name)
}

/// Reset the current workspace to the way it was saved, dropping its changes.
pub fn reset(ui: &mut UiState) -> String {
    let w = find_saved(ui, &ui.workspace.clone()).unwrap_or_default();
    ui.workspace_changes.retain(|x| !x.name.eq_ignore_ascii_case(&w.name));
    apply(ui, &w);
    w.name
}

/// A workspace by name as saved (user workspaces shadow nothing: names are unique across both lists).
pub fn find_saved(ui: &UiState, name: &str) -> Option<Workspace> {
    ui.custom_workspaces
        .iter()
        .find(|w| w.name.eq_ignore_ascii_case(name))
        .cloned()
        .or_else(|| builtins().into_iter().find(|w| w.name.eq_ignore_ascii_case(name)))
}

/// A workspace by name as saved.
pub fn find(ui: &UiState, name: &str) -> Option<Workspace> {
    find_saved(ui, name)
}

/// Every workspace name: built-ins then user workspaces.
pub fn names(ui: &UiState) -> Vec<String> {
    builtins().into_iter().map(|w| w.name).chain(ui.custom_workspaces.iter().map(|w| w.name.clone())).collect()
}

/// Save the current layout as `name` (replacing a user workspace of that name).
pub fn save_as(ui: &mut UiState, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("workspace name can't be empty".into());
    }
    if is_builtin(name) {
        return Err(format!("`{name}` is a built-in workspace; choose another name"));
    }
    // The changes go into the workspace saved, which starts with none; the one left keeps the
    // changes it had (none of these).
    // A user workspace of that name in another case is replaced, as `find_saved` would find it.
    let w = capture(ui, name);
    match ui.custom_workspaces.iter_mut().find(|x| x.name.eq_ignore_ascii_case(name)) {
        Some(x) => *x = w,
        None => ui.custom_workspaces.push(w),
    }
    ui.workspace_changes.retain(|x| !x.name.eq_ignore_ascii_case(name));
    ui.workspace = name.into();
    Ok(())
}

pub fn delete(ui: &mut UiState, name: &str) -> Result<(), String> {
    let before = ui.custom_workspaces.len();
    ui.custom_workspaces.retain(|w| w.name != name);
    if ui.custom_workspaces.len() == before {
        return Err(if is_builtin(name) { format!("`{name}` is built in and can't be deleted") } else { format!("no workspace `{name}`") });
    }
    ui.workspace_changes.retain(|w| w.name != name);
    // The current workspace deleted: Essentials (as it was left) takes over, rather than its layout
    // staying on screen under Essentials's name, to be remembered as Essentials's changes.
    if ui.workspace == name {
        let essentials =
            ui.workspace_changes.iter().find(|w| w.name == ESSENTIALS).cloned().or_else(|| find_saved(ui, ESSENTIALS)).unwrap_or_default();
        apply(ui, &essentials);
    }
    Ok(())
}

pub fn rename(ui: &mut UiState, from: &str, to: &str) -> Result<(), String> {
    let to = to.trim();
    if to.is_empty() || is_builtin(to) || ui.custom_workspaces.iter().any(|w| w.name == to) {
        return Err(format!("`{to}` is not an available name"));
    }
    let w = ui.custom_workspaces.iter_mut().find(|w| w.name == from).ok_or_else(|| format!("no user workspace `{from}`"))?;
    w.name = to.into();
    for w in ui.workspace_changes.iter_mut().filter(|w| w.name == from) {
        w.name = to.into();
    }
    if ui.workspace == from {
        ui.workspace = to.into();
    }
    Ok(())
}

// ---------- menu support (menu_tree has no app access) ----------

crate::shortcut_editor::ui_mirror!(fn names_store() -> Vec<&'static str>);

/// Mirror the user workspace names for the (static) menu tree.
pub fn sync(ui: &UiState) {
    let same = names_store()
        .read()
        .map(|v| v.len() == ui.custom_workspaces.len() && v.iter().zip(&ui.custom_workspaces).all(|(a, b)| *a == b.name))
        .unwrap_or(true);
    if !same {
        if let Ok(mut v) = names_store().write() {
            *v = ui.custom_workspaces.iter().map(|w| crate::shortcut_editor::intern(&w.name)).collect();
        }
        crate::shortcut_editor::GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Window → Workspace submenu items.
pub fn menu_items() -> Vec<crate::menus::Item> {
    use crate::menus::Item;
    let mut v: Vec<Item> =
        builtins().into_iter().map(|w| Item::Cmd(crate::shortcut_editor::intern(&w.name), "window.workspace", json!({"name": w.name}))).collect();
    let custom = names_store().read().map(|v| v.clone()).unwrap_or_default();
    if !custom.is_empty() {
        v.push(Item::Sep);
        v.extend(custom.into_iter().map(|n| Item::Cmd(n, "window.workspace", json!({"name": n}))));
    }
    v.push(Item::Sep);
    v.push(Item::Cmd("Reset Essentials", "window.workspace.reset", Value::Null));
    v.push(Item::Cmd("New Workspace…", "window.workspace.new", Value::Null));
    v.push(Item::Cmd("Manage Workspaces…", "window.workspace.manage", Value::Null));
    v
}

/// The app-bar workspace switcher's menu.
pub fn popup(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    ui.set_min_width(210.0);
    ui.set_max_width(240.0);
    let mut clicked: Option<(&'static str, Value)> = None;
    for it in menu_items() {
        match it {
            crate::menus::Item::Sep => {
                ui.separator();
            }
            crate::menus::Item::Cmd(label, id, p) => {
                // As the Window menu draws it: custom workspace names (and Reset <name>) as they are.
                let label = crate::menus::display_label(app, id, label);
                let text = match crate::menus::checked(app, id, &p) {
                    Some(true) => format!("✓  {label}"),
                    Some(false) => format!("     {label}"),
                    None => format!("     {label}"),
                };
                if ui.add(egui::Button::new(text).min_size(egui::vec2(220.0, 0.0))).clicked() {
                    clicked = Some((id, if p.is_null() { json!({}) } else { p }));
                    ui.close();
                }
            }
            _ => {}
        }
    }
    if let Some((id, p)) = clicked {
        crate::menus::invoke(app, id, p);
    }
}

// ---------- commands ----------

pub fn run_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "window.workspace" => switch(&mut app.ui, &s("name").unwrap_or_default()).map(|name| json!({"workspace": name})),
        "window.workspace.reset" => Ok(json!({"workspace": reset(&mut app.ui)})),
        "window.workspace.new" => match s("name") {
            Some(n) => save_as(&mut app.ui, &n).map(|_| json!({"workspace": n})),
            None => {
                let n = (1..).map(|i| format!("Workspace {i}")).find(|n| find(&app.ui, n).is_none()).unwrap_or_default();
                app.ui.dialog = Some(Dialog::new("newWorkspace", json!({"name": n})));
                Ok(Value::Null)
            }
        },
        "window.workspace.manage" => {
            app.ui.dialog = Some(Dialog::new("manageWorkspaces", json!({"selected": "", "name": ""})));
            Ok(Value::Null)
        }
        "window.workspace.delete" => delete(&mut app.ui, &s("name").unwrap_or_default()).map(|_| Value::Null),
        "window.workspace.rename" => rename(&mut app.ui, &s("name").unwrap_or_default(), &s("to").unwrap_or_default()).map(|_| Value::Null),
        "window.workspace.list" => Ok(json!({"current": app.ui.workspace, "workspaces": names(&app.ui), "custom": app.ui.custom_workspaces})),
        _ => return None,
    };
    Some(r)
}

/// OK in New Workspace / Manage Workspaces.
pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(d) = app.ui.dialog.clone() else { return Err("no dialog open".into()) };
    let r = match d.kind.as_str() {
        "newWorkspace" => save_as(&mut app.ui, &d.str("name")).map(|_| Value::Null),
        _ => Ok(Value::Null),
    };
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let (mut ok, mut cancel) = (false, false);
    let mut action: Option<(&str, Value)> = None;
    let manage = d.kind == "manageWorkspaces";
    crate::dialogs::modal::show(ctx, "Workspace", egui::Id::unique("dialog-workspace"), -40.0, 22, |ui| {
        ui.set_width(340.0);
        crate::dialogs::modal::heading(ui, if manage { tl!("Manage Workspaces") } else { tl!("New Workspace") });
        ui.add_space(12.0);
        if manage {
            egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                ui.set_min_height(140.0);
                ui.set_width(ui.available_width());
                for n in names(&app.ui) {
                    let builtin = is_builtin(&n);
                    let shown = crate::panels::label_or_name(&n, builtin);
                    let text = if builtin { egui::RichText::new(shown).color(t.text_dim) } else { egui::RichText::new(shown).color(t.text) };
                    if ui.selectable_label(d.str("selected") == n, text).clicked() && !builtin {
                        d.fields.insert("selected".into(), json!(n));
                        d.fields.insert("name".into(), json!(n));
                    }
                }
            });
            ui.add_space(8.0);
            let sel = d.str("selected");
            ui.horizontal(|ui| {
                let mut name = d.str("name");
                if ui.add(egui::TextEdit::singleline(&mut name).align(egui::Align2::LEFT_CENTER).desired_width(180.0)).changed() {
                    d.fields.insert("name".into(), json!(name));
                }
                if ui.button(tl!("New")).on_hover_text(tl!("Save the current layout under this name")).clicked() {
                    action = Some(("window.workspace.new", json!({"name": d.str("name")})));
                }
                ui.add_enabled_ui(!sel.is_empty(), |ui| {
                    if ui.button(tl!("Rename")).clicked() {
                        action = Some(("window.workspace.rename", json!({"name": sel, "to": d.str("name")})));
                    }
                    if ui.button(tl!("Delete")).clicked() {
                        action = Some(("window.workspace.delete", json!({"name": sel})));
                    }
                });
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim));
                let mut name = d.str("name");
                let r = ui.add(egui::TextEdit::singleline(&mut name).align(egui::Align2::LEFT_CENTER).desired_width(240.0));
                if r.changed() {
                    d.fields.insert("name".into(), json!(name));
                }
            });
            ui.label(egui::RichText::new(tl!("Saves the current bars, toolbar and panel layout.")).color(t.text_dim).size(11.0));
        }
        ui.add_space(16.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::primary_button(ui, tl!("OK")).clicked() {
                ok = true;
            }
            ui.add_space(8.0);
            if !manage && widgets::secondary_button(ui, tl!("Cancel")).clicked() {
                cancel = true;
            }
        });
    });
    if !manage && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        ok = true;
    }
    app.ui.dialog = Some(d);
    if let Some((id, p)) = action {
        match run_command(app, id, &p) {
            Some(Err(e)) => app.status(e),
            _ => {
                if let Some(d) = app.ui.dialog.as_mut() {
                    d.fields.insert("selected".into(), json!(""));
                }
            }
        }
    }
    if cancel {
        app.ui.dialog = None;
    } else if ok && let Err(e) = confirm(app) {
        app.status(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn essentials_is_the_default_layout() {
        let ui = UiState::default();
        assert_eq!(capture(&ui, ESSENTIALS), builtins()[0]);
        assert_eq!(ui.workspace, ESSENTIALS);
    }

    #[test]
    fn apply_classic_then_reset_to_essentials_keeps_brightness() {
        let mut ui = UiState { brightness: crate::theme::Brightness::Light, ..Default::default() };
        let w = find(&ui, "Essentials Classic").unwrap();
        apply(&mut ui, &w);
        assert!(ui.control_bar && ui.view.rulers && ui.toolbar_advanced);
        assert_eq!(ui.open_panel.as_deref(), Some("color"));
        assert_eq!(ui.workspace, "Essentials Classic");
        let w = find(&ui, "essentials").unwrap();
        apply(&mut ui, &w);
        assert!(!ui.control_bar && !ui.view.rulers);
        assert_eq!(ui.brightness, crate::theme::Brightness::Light);
    }

    #[test]
    fn save_apply_round_trip() {
        let mut ui = UiState { control_bar: true, dock_tab: DockTab::Layers, open_panel: Some("stroke".into()), ..Default::default() };
        save_as(&mut ui, "Mine").unwrap();
        let w = find(&ui, ESSENTIALS).unwrap();
        apply(&mut ui, &w);
        assert!(!ui.control_bar);
        let w = find(&ui, "Mine").unwrap();
        apply(&mut ui, &w);
        assert!(ui.control_bar && ui.dock_tab == DockTab::Layers && ui.open_panel.as_deref() == Some("stroke"));
        // Survives the UiState JSON round trip.
        let back: UiState = serde_json::from_value(serde_json::to_value(&ui).unwrap()).unwrap();
        assert_eq!(back.custom_workspaces, ui.custom_workspaces);
        assert_eq!(back.workspace, "Mine");
    }

    /// Changes made in a workspace come back after switching away and back, and after a restart, until Reset.
    #[test]
    fn a_workspace_keeps_its_changes_across_switches_until_reset() {
        let mut ui = UiState::default();
        save_as(&mut ui, "Görkem").unwrap();
        // Changes made in it after saving (a panel moved, the control bar shown)…
        ui.control_bar = true;
        ui.dock_tab = DockTab::Layers;
        let mut layout = crate::panel_docking::default_layout(&ui);
        if let Some(craft_ui::docking::Node::Split { second, .. }) = layout.root.as_mut()
            && let craft_ui::docking::Node::Dock { columns, .. } = second.as_mut()
        {
            columns[0].groups[0].minimized = true;
        }
        ui.docking = Some(layout.clone());
        // …are still there after switching away and back.
        switch(&mut ui, ESSENTIALS).unwrap();
        assert!(!ui.control_bar && ui.docking.is_none(), "Essentials comes as saved");
        switch(&mut ui, "Görkem").unwrap();
        assert!(ui.control_bar && ui.dock_tab == DockTab::Layers);
        assert_eq!(ui.docking, Some(layout), "its panel layout too");
        // Built-in workspaces remember theirs too.
        switch(&mut ui, ESSENTIALS).unwrap();
        ui.status_bar = false;
        switch(&mut ui, "Görkem").unwrap();
        switch(&mut ui, ESSENTIALS).unwrap();
        assert!(!ui.status_bar);
        // They survive a restart (the preferences' JSON round trip).
        let mut ui: UiState = serde_json::from_value(serde_json::to_value(&ui).unwrap()).unwrap();
        ui = ui.sanitized();
        switch(&mut ui, "Görkem").unwrap();
        assert!(ui.control_bar);
        // Reset Görkem: back to the way it was saved.
        assert_eq!(reset(&mut ui), "Görkem");
        assert!(!ui.control_bar && ui.docking.is_none());
        switch(&mut ui, ESSENTIALS).unwrap();
        switch(&mut ui, "Görkem").unwrap();
        assert!(!ui.control_bar, "a reset workspace stays reset");
        // Renamed, it keeps its changes; deleted, they go.
        ui.control_bar = true;
        switch(&mut ui, ESSENTIALS).unwrap();
        rename(&mut ui, "Görkem", "G").unwrap();
        switch(&mut ui, "G").unwrap();
        assert!(ui.control_bar);
        switch(&mut ui, ESSENTIALS).unwrap();
        delete(&mut ui, "G").unwrap();
        assert!(ui.workspace_changes.iter().all(|w| w.name != "G"));
    }

    /// Deleting the current workspace shows Essentials as it was left, not the deleted one's layout.
    #[test]
    fn deleting_the_current_workspace_switches_to_essentials_without_leaking_its_layout() {
        let mut ui = UiState::default();
        save_as(&mut ui, "Tmp").unwrap();
        ui.control_bar = true;
        delete(&mut ui, "Tmp").unwrap();
        assert_eq!(ui.workspace, ESSENTIALS);
        assert!(!ui.control_bar, "Essentials's layout is shown, not the deleted one's");
        save_as(&mut ui, "Other").unwrap();
        switch(&mut ui, ESSENTIALS).unwrap();
        assert!(!ui.control_bar, "Essentials didn't remember the deleted workspace's layout");
    }

    /// With 64 layouts remembered, the oldest goes and the one just left is kept.
    #[test]
    fn a_full_list_of_remembered_layouts_drops_the_oldest_not_the_newest() {
        let mut ui = UiState::default();
        let names: Vec<String> = (0..70).map(|i| format!("W{i}")).collect();
        for name in &names {
            save_as(&mut ui, name).unwrap();
        }
        for name in &names {
            switch(&mut ui, name).unwrap();
            ui.control_bar = true;
        }
        switch(&mut ui, ESSENTIALS).unwrap();
        assert_eq!(ui.workspace_changes.len(), 64);
        assert!(ui.workspace_changes.iter().all(|w| w.name != "W0"), "the oldest went");
        switch(&mut ui, "W69").unwrap();
        assert!(ui.control_bar, "the layout just left is kept");
    }

    /// Workspace names match whatever their case, as `find` does: "Mine" saved over "mine" replaces
    /// it, and changes made under one spelling come back under the other.
    #[test]
    fn workspace_names_match_whatever_their_case() {
        let mut ui = UiState::default();
        save_as(&mut ui, "mine").unwrap();
        save_as(&mut ui, "Mine").unwrap();
        assert_eq!(ui.custom_workspaces.len(), 1, "one workspace, not two spellings");
        ui.control_bar = true;
        switch(&mut ui, ESSENTIALS).unwrap();
        assert_eq!(switch(&mut ui, "MINE").unwrap(), "Mine");
        assert!(ui.control_bar, "its changes came back");
        // A preferences file with a remembered layout under another spelling, twice: it maps to the
        // saved one, once.
        ui.control_bar = false;
        switch(&mut ui, ESSENTIALS).unwrap();
        let mut odd = capture(&ui, "MINE");
        odd.control_bar = true;
        ui.workspace_changes = vec![odd.clone(), Workspace { name: "mine".into(), ..odd }];
        let mut ui = ui.sanitized();
        assert_eq!(ui.workspace_changes.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["Mine"]);
        switch(&mut ui, "Mine").unwrap();
        assert!(ui.control_bar);
    }

    /// Saving a workspace takes the current layout into it, which then starts with no changes.
    #[test]
    fn saving_a_workspace_moves_the_changes_into_it() {
        let mut ui = UiState { control_bar: true, ..Default::default() };
        save_as(&mut ui, "Mine").unwrap();
        switch(&mut ui, ESSENTIALS).unwrap();
        assert!(!ui.control_bar, "Essentials stays as saved: the changes went into Mine");
        switch(&mut ui, "Mine").unwrap();
        assert!(ui.control_bar);
        // Saving again under the same name takes the current layout and starts with no changes.
        ui.status_bar = false;
        save_as(&mut ui, "Mine").unwrap();
        assert!(ui.workspace_changes.iter().all(|w| w.name != "Mine"));
        ui.status_bar = true;
        assert_eq!(reset(&mut ui), "Mine");
        assert!(!ui.status_bar);
    }

    #[test]
    fn builtin_names_are_protected() {
        let mut ui = UiState::default();
        assert!(save_as(&mut ui, "Painting").is_err());
        assert!(save_as(&mut ui, "  ").is_err());
        assert!(delete(&mut ui, "Essentials").is_err());
        save_as(&mut ui, "A").unwrap();
        assert!(rename(&mut ui, "A", "Web").is_err());
        rename(&mut ui, "A", "B").unwrap();
        assert_eq!(ui.workspace, "B");
        delete(&mut ui, "B").unwrap();
        assert_eq!(ui.workspace, ESSENTIALS);
    }

    #[test]
    fn all_requested_workspaces_exist() {
        let ui = UiState::default();
        for n in ["Essentials", "Essentials Classic", "Painting", "Typography", "Layout", "Tracing", "Web", "Automation", TOUCH] {
            assert!(find(&ui, n).is_some(), "{n}");
        }
    }

    #[test]
    fn touch_workspace_preference_switches_and_restores() {
        use crate::{Services, VectorcraftApp};
        use vectorcraft_engine::Session;

        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.ui.workspace = "Typography".into();
        app.session.prefs.touch_workspace = true;
        sync_touch_workspace(&mut app, true);
        assert_eq!(app.ui.workspace, TOUCH);
        assert_eq!(app.ui.touch_workspace_restore.as_deref(), Some("Typography"));
        assert!(app.ui.control_bar && app.ui.open_panel.as_deref() == Some("align"));
        app.session.prefs.touch_workspace = false;
        sync_touch_workspace(&mut app, true);
        assert_eq!(app.ui.workspace, "Typography");
        assert!(app.ui.touch_workspace_restore.is_none());
    }

    #[test]
    fn saved_workspaces_restore_dock_width() {
        let mut ui = UiState { dock_width: Some(420.0), ..Default::default() };
        save_as(&mut ui, "Wide").unwrap();
        ui.dock_width = Some(260.0);
        save_as(&mut ui, "Narrow").unwrap();
        let wide = find(&ui, "Wide").unwrap();
        apply(&mut ui, &wide);
        assert_eq!(ui.dock_width, Some(420.0));
        let narrow = find(&ui, "Narrow").unwrap();
        apply(&mut ui, &narrow);
        assert_eq!(ui.dock_width, Some(260.0));
        let back: UiState = serde_json::from_value(serde_json::to_value(&ui).unwrap()).unwrap();
        assert_eq!(back.custom_workspaces[1].dock_width, Some(260.0));
    }
}
