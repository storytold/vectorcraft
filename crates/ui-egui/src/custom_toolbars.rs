//! Window › Toolbars › New Toolbar (#1759): toolbars the user makes and names. Each toolbar is a
//! `toolbar:<id>` panel of the dock layout that floats when it is made; the dock draws it, moves it,
//! docks it into columns and floats it again (`panel_docking`, `toolbar::rail_view`). This module is
//! what the dock does not know: the toolbars' records (`CustomToolbar`, kept with the workspace they
//! were made in), their names, the commands that make, copy, rename, delete, show and list them, the
//! Add Tool… menu in a toolbar's footer, and the New Toolbar and Manage Toolbars dialogs.
//!
//! The ••• button at a toolbar's foot opens the All Tools drawer beside it, and tools dragged from the
//! drawer fill it (`edit_toolbar`, with the `toolbar` param of its commands). The + button above it
//! opens Add Tool…, a menu of every tool where a click adds the tool at the end, and whose Remove Tool
//! items take tools out: VectorCraft's own way to fill a toolbar without a drag. Each toolbar is listed
//! under Window › Toolbars, where its item shows or hides it. Window › Toolbars › Manage Toolbars (also
//! in the drawer's menu) renames, copies and deletes them.
//!
//! Behavior from the public documentation:
//! <https://helpx.adobe.com/illustrator/using/tools.html> (Create and manage toolbars: New Toolbar
//! from Window › Toolbars or the drawer's flyout menu, a name and OK make a blank toolbar, its Edit
//! Toolbar button opens the drawer to add tools, and a toolbar stays in the workspace it is made in.
//! Manage toolbars: Manage Toolbars from Window › Toolbars or the drawer's flyout menu; a new name
//! typed for the selected toolbar renames it, + copies the selected toolbar or, with none selected,
//! makes a new one, and Delete deletes it).

use std::collections::BTreeSet;

use craft::behavior::IconId;
use craft::behavior::dock::{Action, DockEdge};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{PopupKind, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::dialog::DialogResult;
use craft::ui::display::Divider;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::patterns::form_dialogs::{
    DialogFields, FieldValue, HostedListManager, HostedParamDialog, ListManagerAction, ListManagerChoice, ListManagerEntry, ListManagerWords, Param,
    ParamDialog, ParamKind, ParamResult,
};
use craft::ui::rails::RailFooter;
use craft::ui::runtime::commands::CommandRef;
use egui::pos2;
use serde_json::{Value, json};
use vectorcraft_tools::tool_info;

use crate::state::{CustomToolbar, UiState};
use crate::toolbar::{Owner, strip_id};
use crate::{VectorcraftApp, panel_docking};

/// The most toolbars a workspace keeps.
pub(crate) const MAX: usize = 64;
/// The most characters a toolbar's name has.
const MAX_NAME: usize = 200;
/// Where the first toolbar made floats, right of the Tools panel; the next ones go lower.
const NEW_AT: [f32; 2] = [96.0, 120.0];
/// The kind every Manage Toolbars entry has: all of them rename in place.
const ENTRY_KIND: u32 = 0;
/// The key of New Toolbar's dialog.
const NEW_KEY: &str = "window.toolbar.new";
/// The key of Manage Toolbars' dialog.
const MANAGE_KEY: &str = "window.toolbar.manage";
/// The id of Manage Toolbars' copy action.
const DUPLICATE: &str = "duplicate";
/// The id the framework gives the delete action ([`ListManagerAction::delete`]).
const DELETE: &str = "delete";

/// A toolbar's name without leading and trailing spaces, in lowercase. Names with the same key are
/// the same name: lookups, the check that a name is free and sanitizing compare keys.
pub(crate) fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The dock panel of the toolbar with id `id`.
pub(crate) fn panel_id(id: u32) -> String {
    format!("toolbar:{id}")
}

/// Whether `bar` is on, as Window › Toolbars checks it: its panel is in the layout, though Tab may hide it for now.
pub(crate) fn is_on(ui: &UiState, bar: &CustomToolbar) -> bool {
    panel_docking::layout_of(ui).contains(&panel_id(bar.id))
}

/// The toolbar named `name`, whatever its case.
pub(crate) fn find<'a>(ui: &'a UiState, name: &str) -> Option<&'a CustomToolbar> {
    let k = key(name);
    ui.custom_toolbars.iter().find(|b| key(&b.name) == k)
}

pub(crate) fn find_mut<'a>(ui: &'a mut UiState, name: &str) -> Option<&'a mut CustomToolbar> {
    let k = key(name);
    ui.custom_toolbars.iter_mut().find(|b| key(&b.name) == k)
}

/// The `toolbar` param: the name of a toolbar made with New Toolbar, as it was given (none or
/// null: the Tools panel's toolbar).
pub(crate) fn target(ui: &UiState, p: &Value) -> Result<Option<String>, String> {
    match p.get("toolbar") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let name = v.as_str().ok_or("toolbar (a toolbar's name) must be a string")?;
            find(ui, name).map(|b| Some(b.name.clone())).ok_or_else(|| format!("no toolbar named `{name}`"))
        }
    }
}

/// The toolbar the open All Tools drawer edits: one made with New Toolbar, or none for the Tools
/// panel's toolbar (and while the drawer is closed).
pub(crate) fn drawer_target(ui: &UiState) -> Option<&str> {
    ui.drawer_toolbar.as_deref().filter(|_| ui.tools_drawer)
}

/// The lowest id from 1 that `taken` does not hold.
fn lowest_free(taken: impl Fn(u32) -> bool) -> u32 {
    (1..).find(|id| !taken(*id)).unwrap_or(1)
}

/// The id a toolbar made now gets.
fn next_id(bars: &[CustomToolbar]) -> u32 {
    lowest_free(|id| bars.iter().any(|b| b.id == id))
}

/// `name` as a toolbar's name: trimmed, not empty, at most [`MAX_NAME`] characters and not the name
/// of any toolbar but the one at `except`.
fn valid_name(ui: &UiState, name: &str, except: Option<usize>) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("toolbar name can't be empty".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("a toolbar name has at most {MAX_NAME} characters"));
    }
    if ui.custom_toolbars.iter().enumerate().any(|(i, b)| Some(i) != except && key(&b.name) == key(name)) {
        return Err(format!("there is already a toolbar named `{name}`"));
    }
    Ok(name.to_string())
}

/// The first free "Toolbar N".
fn blank_name(ui: &UiState) -> String {
    (1..).map(|i| format!("Toolbar {i}")).find(|n| find(ui, n).is_none()).unwrap_or_default()
}

/// Floats the panel of toolbar `id` `width` points wide (the dock's float width for it when `width` is
/// not positive), at the first step from `first` of the cascade
/// from `from` (16 right and 24 down a step) where no other toolbar's window sits, and shows the
/// panels, which ends a Tab hide.
fn float_new(app: &mut VectorcraftApp, id: u32, from: [f32; 2], first: u32, width: f32) -> Result<(), String> {
    let layout = panel_docking::layout_of(&app.ui);
    let at = |n: u32| pos2(from[0] + 16.0 * n as f32, from[1] + 24.0 * n as f32);
    let sits = |p: egui::Pos2| {
        let others = app.ui.custom_toolbars.iter().filter(|b| b.id != id);
        others.filter_map(|b| panel_docking::floating_window(layout, &panel_id(b.id))).any(|w| w.rect.min.distance(p) < 0.5)
    };
    let n = (first..=first + MAX as u32).find(|&n| !sits(at(n))).unwrap_or(first);
    panel_docking::place_floating(app, &panel_id(id), at(n), width)?;
    panel_docking::show_panels(app);
    Ok(())
}

/// Make a blank toolbar named `name`, floating and shown; returns its name.
pub(crate) fn create(app: &mut VectorcraftApp, name: &str) -> Result<String, String> {
    let name = valid_name(&app.ui, name, None)?;
    if app.ui.custom_toolbars.len() >= MAX {
        return Err(format!("a workspace keeps at most {MAX} toolbars"));
    }
    let id = next_id(&app.ui.custom_toolbars);
    app.ui.custom_toolbars.push(blank(id, name.clone()));
    float_new(app, id, NEW_AT, 0, 0.0)?;
    Ok(name)
}

/// A toolbar with no tools, called `name`, whose panel is `toolbar:<id>`.
fn blank(id: u32, name: String) -> CustomToolbar {
    CustomToolbar {
        id,
        name,
        slots: Vec::new(),
        last_used: Default::default(),
        shown: true,
        pos: [0.0; 2],
        dock: None,
        dock_right: false,
        slot_tool: Default::default(),
        double: false,
        floating: Vec::new(),
    }
}

/// Where the toolbar named `name` is in the list, whatever its case.
fn index(ui: &UiState, name: &str) -> Result<usize, String> {
    let k = key(name);
    ui.custom_toolbars.iter().position(|b| key(&b.name) == k).ok_or_else(|| format!("no toolbar named `{name}`"))
}

/// The All Tools drawer beside the toolbar named `old` closes when it is deleted or hidden (`new` is
/// `None`) or renamed; a new name that changes only the case leaves it open on the new name.
fn close_drawer(ui: &mut UiState, old: &str, new: Option<&str>) {
    if drawer_target(ui).map(key) != Some(key(old)) {
        return;
    }
    match new.filter(|n| key(n) == key(old)) {
        Some(same) => ui.drawer_toolbar = Some(same.to_string()),
        None => {
            ui.tools_drawer = false;
            ui.drawer_toolbar = None;
        }
    }
}

/// Closes the panels `panels`. One that is hidden is not in the layout, which is the state closing
/// asks for, so a refusal is not an error.
fn close_panels(app: &mut VectorcraftApp, panels: Vec<String>) {
    let ctx = app.ui_ctx.clone();
    for panel in panels {
        panel_docking::apply(app, ctx.as_ref(), vec![Action::close(panel)]).ok();
    }
}

/// Give the toolbar named `name` the name `to`; returns the new name. Its panel keeps its id, so the
/// window and anything dragging it stay.
pub(crate) fn rename(ui: &mut UiState, name: &str, to: &str) -> Result<String, String> {
    let k = index(ui, name);
    let to = valid_name(ui, to, k.as_ref().ok().copied())?;
    let k = k?;
    let old = ui.custom_toolbars.get(k).map(|b| b.name.clone()).unwrap_or_default();
    close_drawer(ui, &old, Some(&to));
    if let Some(b) = ui.custom_toolbars.get_mut(k) {
        b.name.clone_from(&to);
    }
    Ok(to)
}

/// A copy of the toolbar named `name`, with its tools, floating a little below its window (further
/// down while an earlier copy is there) and called "`name` Copy" (then "`name` Copy 2"…); without
/// `name`, a blank toolbar called "Toolbar N". Returns the new toolbar's name.
pub(crate) fn duplicate(app: &mut VectorcraftApp, name: Option<&str>) -> Result<String, String> {
    let Some(name) = name else {
        let name = blank_name(&app.ui);
        return create(app, &name);
    };
    let k = index(&app.ui, name)?;
    if app.ui.custom_toolbars.len() >= MAX {
        return Err(format!("a workspace keeps at most {MAX} toolbars"));
    }
    let Some(mut copy) = app.ui.custom_toolbars.get(k).cloned() else { return Err(format!("no toolbar named `{name}`")) };
    // A long name is shortened so that "Copy" and its number fit in MAX_NAME characters.
    let stem: String = copy.name.chars().take(MAX_NAME - " Copy 64".len()).collect();
    let base = format!("{} Copy", stem.trim_end());
    let ui = &app.ui;
    copy.name = std::iter::once(base.clone()).chain((2..).map(|i| format!("{base} {i}"))).find(|n| find(ui, n).is_none()).unwrap_or_default();
    let source = panel_docking::floating_window(panel_docking::layout_of(&app.ui), &panel_id(copy.id)).map(|w| w.rect);
    copy.id = next_id(&app.ui.custom_toolbars);
    let (id, name) = (copy.id, copy.name.clone());
    app.ui.custom_toolbars.push(copy);
    match source {
        Some(r) => float_new(app, id, [r.min.x, r.min.y], 1, r.width())?,
        None => float_new(app, id, NEW_AT, 0, 0.0)?,
    }
    Ok(name)
}

/// Delete the toolbar named `name`, with the groups torn off it.
pub(crate) fn delete(app: &mut VectorcraftApp, name: &str) -> Result<(), String> {
    let k = index(&app.ui, name)?;
    let Some(bar) = app.ui.custom_toolbars.get(k) else { return Err(format!("no toolbar named `{name}`")) };
    let (id, bar_name) = (bar.id, bar.name.clone());
    let mut panels: Vec<String> = crate::toolbar::strips(app, Some(&bar_name))
        .iter()
        .filter_map(|tools| tools.first())
        .map(|first| strip_id(&Owner::Toolbar(id), first))
        .collect();
    panels.push(panel_id(id));
    close_drawer(&mut app.ui, &bar_name, None);
    close_panels(app, panels);
    app.ui.custom_toolbars.remove(k);
    Ok(())
}

/// Keep named toolbars, each name once whatever its case and at most [`MAX_NAME`] characters, of
/// known tools, each once in its toolbar, each group's tool chosen last only when the group holds it,
/// and each with an id of its own: one saved without an id or repeating an earlier toolbar's gets
/// the lowest id nobody holds (a hand-edited preferences file or workspace).
pub(crate) fn sanitize(bars: &mut Vec<CustomToolbar>) {
    let mut names = BTreeSet::new();
    bars.retain_mut(|b| {
        b.name = b.name.trim().to_string();
        let mut seen = BTreeSet::new();
        for s in &mut b.slots {
            s.retain(|id| tool_info(id).is_some() && seen.insert(id.clone()));
        }
        b.slots.retain(|s| !s.is_empty());
        let old = std::mem::take(&mut b.last_used);
        for (first, slot) in b.slots.iter().filter_map(|s| Some((s.first()?, s))) {
            let at = crate::toolbar::slot_id(first);
            if let Some(tool) = old.get(at).filter(|t| slot.contains(t)) {
                b.last_used.set(at, tool.clone());
            }
        }
        !b.name.is_empty() && b.name.chars().count() <= MAX_NAME && names.insert(key(&b.name))
    });
    bars.truncate(MAX);
    let mut held = BTreeSet::new();
    let needy: Vec<usize> = bars.iter().enumerate().filter(|(_, b)| b.id == 0 || !held.insert(b.id)).map(|(k, _)| k).collect();
    for k in needy {
        let id = lowest_free(|id| held.contains(&id));
        held.insert(id);
        if let Some(b) = bars.get_mut(k) {
            b.id = id;
        }
    }
}

/// Close the strips torn off toolbar `id` whose group it no longer holds (a group that is gone, now
/// starts with another tool or holds one tool alone goes back); the others show the group's tools as
/// they are, since a strip reads them from the toolbar's slot.
pub(crate) fn keep_strips(app: &mut VectorcraftApp, id: u32) {
    let Some(bar) = app.ui.custom_toolbars.iter().find(|b| b.id == id) else { return };
    let holds = |first: &str| bar.slots.iter().any(|s| s.len() > 1 && s.first().map(String::as_str) == Some(first));
    let gone: Vec<String> = crate::toolbar::strips(app, Some(&bar.name))
        .iter()
        .filter_map(|tools| tools.first())
        .filter(|first| !holds(first))
        .map(|first| strip_id(&Owner::Toolbar(id), first))
        .collect();
    close_panels(app, gone);
}

/// Shows or hides the panel of toolbar `name` (none: toggles); returns whether it shows. Showing ends
/// a Tab hide; hiding closes the drawer that edits it.
fn set_shown(app: &mut VectorcraftApp, name: &str, show: Option<bool>) -> Result<bool, String> {
    let bar = find(&app.ui, name).ok_or_else(|| format!("no toolbar named `{name}`"))?;
    let (id, name) = (bar.id, bar.name.clone());
    let panel = panel_id(id);
    let shown = panel_docking::visible(app, &panel);
    let show = show.unwrap_or(!shown);
    if show && !shown {
        // A hidden panel comes back where it was.
        app.run("window.panel.dock", json!({ "panel": panel }))?;
        crate::toolbar::strips_follow(app, &Owner::Toolbar(id), true);
    } else if !show {
        crate::toolbar::strips_follow(app, &Owner::Toolbar(id), false);
        close_panels(app, vec![panel]);
        close_drawer(&mut app.ui, &name, None);
    }
    if show {
        panel_docking::show_panels(app);
    }
    Ok(show)
}

/// Opens New Toolbar: a name box whose OK is held while the name is refused, the refusal shown under
/// the row.
fn open_new(app: &mut VectorcraftApp) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let fields = DialogFields::from_pairs([("name", blank_name(&app.ui))]);
    let dialog = HostedParamDialog::new(&ctx, NEW_KEY, fields, |_| {
        ParamDialog::new(NEW_KEY, tl!("New Toolbar"), vec![Param::new("name", tl!("Name:"), ParamKind::text(""))])
    });
    crate::dialogs::host::open_held(app, dialog, |app, answer: &ParamResult| {
        let name = match answer.fields.get("name") {
            Some(FieldValue::Text(name)) => name.clone(),
            _ => String::new(),
        };
        create(app, &name).map(|_| ())
    })
    .ok_or("no window")?;
    Ok(())
}

/// The id Manage Toolbars' dialog has on the host.
fn manage_id() -> craft::ui::dialog::DialogId {
    craft::ui::dialog::dialog_id(MANAGE_KEY)
}

/// Opens Manage Toolbars; one already open stays.
fn open_manage(app: &mut VectorcraftApp) -> Result<(), String> {
    let dialogs = app.dialogs().ok_or("no window")?;
    let words = ListManagerWords::default();
    // Copy is a row action that takes the selected entry like Edit does, so it starts from it.
    let mut copy = ListManagerAction::edit(&words);
    copy.id = DUPLICATE.into();
    copy.label = tl!("Duplicate Toolbar").into();
    let actions = vec![copy.keeps_open(), ListManagerAction::delete(&words), ListManagerAction::create(&words).keeps_open()];
    let dialog = HostedListManager::new(MANAGE_KEY, tl!("Manage Toolbars"), tl!("Name:"), actions).entries(entries(&app.ui)).renames(ENTRY_KIND);
    dialogs.with(|host| {
        if host.get_by_id::<HostedListManager>(manage_id()).is_none() {
            host.open(dialog);
        }
    });
    Ok(())
}

/// The rows of Manage Toolbars: one per toolbar, by id.
fn entries(ui: &UiState) -> Vec<ListManagerEntry> {
    ui.custom_toolbars.iter().map(|b| ListManagerEntry::new(b.id.to_string(), b.name.clone()).kind(ENTRY_KIND)).collect()
}

/// Once per frame: Manage Toolbars shows the toolbars as they are now, and the action pressed in it
/// runs as the command of the same name.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let rows = entries(&app.ui);
    // A choice that kept the dialog open waits on it; the one that closed it is its answer.
    let choices = dialogs.with(|host| {
        let kept = host.get_mut_by_id::<HostedListManager>(manage_id()).and_then(|d| {
            d.set_entries(rows);
            d.take_choice()
        });
        [kept, host.take_by_id::<HostedListManager>(manage_id()).and_then(DialogResult::ok)]
    });
    for choice in choices.into_iter().flatten() {
        run_choice(app, &choice);
    }
}

/// Runs the Manage Toolbars action `choice` as the command of the same name.
fn run_choice(app: &mut VectorcraftApp, choice: &ListManagerChoice) {
    let name = choice
        .entry
        .as_deref()
        .and_then(|id| id.parse::<u32>().ok())
        .and_then(|id| app.ui.custom_toolbars.iter().find(|b| b.id == id))
        .map(|b| b.name.clone());
    let (command, params) = if choice.is_rename() {
        ("window.toolbar.rename", json!({ "name": name, "to": choice.name }))
    } else {
        match choice.action.as_str() {
            DUPLICATE => ("window.toolbar.duplicate", json!({ "name": name })),
            DELETE => ("window.toolbar.delete", json!({ "name": name })),
            _ => ("window.toolbar.new", json!({ "name": choice.name })),
        }
    };
    if let Err(e) = app.run(command, params) {
        app.status(e);
    }
}

/// The commands of the toolbars made with New Toolbar; `None` for other ids.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "window.toolbar.new" => match p.get("name").filter(|v| !v.is_null()) {
            Some(name) => match name.as_str() {
                Some(name) => create(app, name).map(|name| json!({ "toolbar": name })),
                None => Err("name (a toolbar's name) must be a string".into()),
            },
            None => open_new(app).map(|()| Value::Null),
        },
        "window.toolbar.custom" => (|| {
            let show = crate::menus::opt_bool(p, "show")?;
            let name = p.get("name").and_then(Value::as_str).ok_or("name (a toolbar's name) is required")?;
            set_shown(app, name, show).map(|shown| json!(shown))
        })(),
        "window.toolbar.manage" => open_manage(app).map(|()| Value::Null),
        "window.toolbar.rename" => (|| {
            let name = p.get("name").and_then(Value::as_str).ok_or("name (a toolbar's name) is required")?;
            let to = p.get("to").and_then(Value::as_str).ok_or("to (the new name) is required")?;
            rename(&mut app.ui, name, to).map(|name| json!({ "toolbar": name }))
        })(),
        "window.toolbar.duplicate" => (|| {
            let name = match p.get("name").filter(|v| !v.is_null()) {
                Some(v) => Some(v.as_str().ok_or("name must be a toolbar's name")?),
                None => None,
            };
            duplicate(app, name).map(|name| json!({ "toolbar": name }))
        })(),
        "window.toolbar.delete" => (|| {
            let name = p.get("name").and_then(Value::as_str).ok_or("name (a toolbar's name) is required")?;
            delete(app, name).map(|_| Value::Null)
        })(),
        "window.toolbar.list" => {
            let layout = panel_docking::layout_of(&app.ui);
            Ok(json!(
                app.ui
                    .custom_toolbars
                    .iter()
                    .map(|b| {
                        let panel = panel_id(b.id);
                        let place = panel_docking::column_of(layout, &panel);
                        let docked = place.as_ref().and_then(|(at, _)| at.as_ref());
                        let side = if matches!(docked, Some((DockEdge::End, _))) { "right" } else { "left" };
                        let double = place.as_ref().is_some_and(|(_, column)| column.steps == Some(2));
                        json!({
                            "name": b.name,
                            "shown": panel_docking::visible(app, &panel),
                            "double": double,
                            "slots": b.slots,
                            "floating": docked.is_none(),
                            "side": side,
                            "order": docked.map(|(_, n)| *n),
                        })
                    })
                    .collect::<Vec<_>>()
            ))
        }
        _ => return None,
    })
}

// ---------- the menu ----------

/// Window › Toolbars: Advanced, Double Column and Compact Tools, the toolbars made with New
/// Toolbar (`names`, each checked while it shows), New Toolbar… and Manage Toolbars….
pub fn menu_items(names: &[String]) -> Vec<crate::menus::Node> {
    use crate::menus::{Sep, c, named};
    let mut v =
        vec![c("Advanced", "window.toolbarAdvanced"), c("Double Column", "window.toolbarColumns"), c("Compact Tools", "window.toolbarCompact"), Sep];
    if !names.is_empty() {
        v.extend(names.iter().map(|n| named(n.as_str(), "window.toolbar.custom", json!({ "name": n }))));
        v.push(Sep);
    }
    v.push(c("New Toolbar…", "window.toolbar.new"));
    v.push(c("Manage Toolbars…", "window.toolbar.manage"));
    v
}

// ---------- the toolbars ----------

/// The tool a slot's button on `bar` shows, of the slot's known `tools`: the active tool when the
/// slot holds it, else the one chosen last from the slot's group, else its first; none for a slot
/// with no known tool.
pub(crate) fn shown_tool(bar: &CustomToolbar, active: &'static str, tools: &[&'static str]) -> Option<&'static str> {
    let &first = tools.first()?;
    if tools.contains(&active) {
        return Some(active);
    }
    Some(bar.last_used.get(crate::toolbar::slot_id(first)).and_then(|id| tools.iter().copied().find(|t| *t == id.as_str())).unwrap_or(first))
}

/// Add Tool…, in the footer of the toolbar with id `id`: the + button opens a menu where, under Remove
/// Tool, a click takes one of the toolbar's tools out; then every tool under the All Tools drawer's
/// headings, where a click adds one at the end (dimmed: in the toolbar already). `measured`: the rail
/// measures its footer, which draws the button alone.
pub(crate) fn add_menu(app: &VectorcraftApp, ui: &mut egui::Ui, id: u32, _footer: RailFooter, measured: bool) {
    let Some(bar) = app.ui.custom_toolbars.iter().find(|b| b.id == id) else { return };
    let held: Vec<&'static str> = bar.slots.iter().flatten().filter_map(|id| tool_info(id).map(|t| t.id)).collect();
    let plus = IconButton::new(IconId::new("plus")).label(tl!("Add Tool…")).popup(PopupKind::Menu).show(ui).response;
    // The rail measures its footer too: the menu opens from the button the rail shows.
    if measured || craft::ui::runtime::measure::active(ui.ctx()) {
        return;
    }
    let command = |verb: &str, tool: &str| CommandRef::with_params(verb, json!({ "tool": tool, "toolbar": bar.name }));
    Menu::new(tl!("Add Tool…")).opened_by(&plus, Trigger::Click).show(ui, |ui, _| {
        if !held.is_empty() {
            MenuItem::heading(crate::i18n::colon(tl!("Remove Tool"))).show(ui);
            for tool in held.iter().filter_map(|id| tool_info(id)) {
                MenuItem::command(command("toolbar.removeTool", tool.id)).label(tool.label).show(ui);
            }
            Divider::horizontal().show(ui);
        }
        for (cat, tools) in crate::edit_toolbar::sections() {
            MenuItem::heading(crate::i18n::colon(tl!(cat))).show(ui);
            for tool in tools {
                MenuItem::command(command("toolbar.addTool", tool.id)).label(tool.label).enabled(!held.contains(&tool.id)).show(ui);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;

    use super::*;
    use crate::menus::Node;

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(Session::new(), Default::default())
    }

    /// The items of Window › Toolbars: (label, command id, params).
    fn toolbars_menu(app: &VectorcraftApp) -> Vec<(String, String, Value)> {
        let window = crate::menus::menu_tree_named(true, &crate::menus::Listed::of(app))
            .into_iter()
            .find(|(m, _)| *m == "Window")
            .map(|(_, items)| items)
            .unwrap();
        let Some(items) = window.into_iter().find_map(|n| match n {
            Node::Submenu { label, children, .. } if label == "Toolbars" => Some(children),
            _ => None,
        }) else {
            panic!("no Toolbars submenu")
        };
        items.into_iter().filter_map(|n| if let Node::Item(i) = n { Some((i.label, i.command.id, i.command.params)) } else { None }).collect()
    }

    /// One frame of the whole window, `height` points tall, with `events` as its input.
    fn window_frame_with(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, height: f32, events: Vec<egui::Event>) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, height));
        let raw = egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() };
        crate::testing::app_frame(app, ctx, raw);
    }

    /// One frame of the whole window, `height` points tall.
    fn window_frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, height: f32) {
        window_frame_with(app, ctx, time, height, vec![]);
    }

    /// A press of Tab, which hides and shows the panels, over three frames from `time`.
    fn tab(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) {
        for (k, pressed) in [true, false].into_iter().enumerate() {
            let key = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
            window_frame_with(app, ctx, time + 0.1 * k as f64, 800.0, vec![key]);
        }
        window_frame(app, ctx, time + 0.3, 800.0);
    }

    fn shown(app: &VectorcraftApp, name: &str) -> bool {
        panel_docking::visible(app, &panel_id(find(&app.ui, name).unwrap().id))
    }

    /// New Toolbar named "Mine" is listed under Window › Toolbars, checked while it shows; its item
    /// hides and shows it. Without a name, a dialog asks for one.
    #[test]
    fn a_new_toolbar_is_listed_under_window_toolbars() {
        let mut app = app();
        let ctx = crate::testing::ui_context();
        let menu = toolbars_menu(&app);
        let last: Vec<_> = menu.iter().rev().take(2).map(|(l, id, _)| (l.as_str(), id.as_str())).collect();
        assert_eq!(last, [("Manage Toolbars…", "window.toolbar.manage"), ("New Toolbar…", "window.toolbar.new")]);
        assert_eq!(app.run("window.toolbar.new", json!({"name": " Mine "})).unwrap(), json!({"toolbar": "Mine"}));
        let menu = toolbars_menu(&app);
        let item = menu.iter().find(|(l, ..)| l == "Mine").expect("Mine is listed");
        assert_eq!((item.1.as_str(), &item.2), ("window.toolbar.custom", &json!({"name": "Mine"})));
        assert_eq!(crate::menus::checked(&app, &item.1, &item.2), Some(true));
        assert_eq!(crate::menus::display_label(&app, &item.1, &item.0), "Mine", "listed by its own name");
        // A blank toolbar, floating, shown.
        assert!(find(&app.ui, "mine").unwrap().slots.is_empty() && shown(&app, "Mine"));
        // Its item hides it and shows it again.
        assert_eq!(app.run("window.toolbar.custom", json!({"name": "Mine"})).unwrap(), json!(false));
        assert!(!shown(&app, "Mine"));
        assert_eq!(crate::menus::checked(&app, &item.1, &item.2), Some(false));
        assert_eq!(app.run("window.toolbar.custom", json!({"name": "MINE", "show": true})).unwrap(), json!(true));
        assert!(shown(&app, "Mine"));
        // Names: not empty, not taken whatever the case.
        assert!(app.run("window.toolbar.new", json!({"name": "  "})).unwrap_err().contains("empty"));
        assert!(app.run("window.toolbar.new", json!({"name": "mine"})).unwrap_err().contains("already"));
        // Whatever the case outside ASCII too: one name, one toolbar.
        app.run("window.toolbar.new", json!({"name": "É"})).unwrap();
        assert!(app.run("window.toolbar.new", json!({"name": "é"})).unwrap_err().contains("already"));
        assert!(app.run("window.toolbar.custom", json!({"name": "é", "show": true})).is_ok());
        app.run("window.toolbar.delete", json!({"name": "É"})).unwrap();
        assert!(app.run("window.toolbar.custom", json!({"name": "Nope"})).unwrap_err().contains("Nope"));
        // A name is looked up without its leading and trailing spaces, as it was made.
        assert_eq!(app.run("window.toolbar.custom", json!({"name": " Mine ", "show": true})).unwrap(), json!(true));
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine "})).is_ok());
        assert!(app.run("toolbar.removeTool", json!({"tool": "pen", "toolbar": "Mine"})).is_ok());
        // A name that is not a string is reported as one.
        assert!(app.run("window.toolbar.new", json!({"name": 5})).unwrap_err().contains("must be a string"));
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": 5})).unwrap_err().contains("must be a string"));
        // Without a name: New Toolbar opens on the dialog host, and makes no toolbar by itself.
        window_frame(&mut app, &ctx, 0.0, 800.0);
        assert_eq!(app.run("window.toolbar.new", json!({})).unwrap(), Value::Null);
        let new_open =
            |d: craft::ui::shell::WindowDialogs| d.with(|h| h.get_by_id::<HostedParamDialog>(craft::ui::dialog::dialog_id(NEW_KEY)).is_some());
        assert!(app.dialogs().is_some_and(new_open));
        assert_eq!(app.ui.custom_toolbars.len(), 1);
        app.run("window.toolbar.new", json!({"name": "Toolbar 1"})).unwrap();
        let names: Vec<String> = toolbars_menu(&app).into_iter().filter(|(_, id, _)| *id == "window.toolbar.custom").map(|(l, ..)| l).collect();
        assert_eq!(names, ["Mine", "Toolbar 1"]);
        let list = app.run("window.toolbar.list", json!({})).unwrap();
        let blank = |name: &str| json!({"name": name, "shown": true, "double": false, "slots": [], "floating": true, "side": "left", "order": null});
        assert_eq!(list, json!([blank("Mine"), blank("Toolbar 1")]));
    }

    /// Switching workspaces while the drawer edits a toolbar of the workspace left closes the
    /// drawer, so it names no toolbar that the new workspace lacks.
    #[test]
    fn switching_workspaces_closes_the_drawer_on_a_toolbar_left() {
        let mut app = app();
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("window.editToolbar", json!({"toolbar": "Mine", "open": true})).unwrap();
        assert_eq!(drawer_target(&app.ui), Some("Mine"));
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(!app.ui.tools_drawer && app.ui.drawer_toolbar.is_none());
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).is_err(), "Mine is not in Essentials");
    }

    /// An app that loaded `app`'s saved state, as after a restart.
    fn reloaded(app: &VectorcraftApp) -> VectorcraftApp {
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        let mut again = self::app();
        again.ui = prefs.sanitized();
        again
    }

    /// Tools added to a toolbar made with New Toolbar leave the Tools panel's toolbar as it was, and
    /// the toolbar comes back with them after switching workspaces and after a restart.
    #[test]
    fn tools_added_to_a_new_toolbar_survive_a_workspace_reload() {
        let mut app = app();
        let shipped = crate::toolbar::slots(&app);
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        let r = app.run("toolbar.addTool", json!({"tool": "curvature", "toolbar": "mine", "slot": 0, "place": "group"})).unwrap();
        assert_eq!(r, json!([["pen", "curvature"]]));
        let r = app.run("toolbar.addTool", json!({"tool": "artboard", "toolbar": "Mine", "slot": 0})).unwrap();
        assert_eq!(r, json!([["artboard"], ["pen", "curvature"]]));
        assert_eq!(crate::toolbar::slots(&app), shipped, "the Tools panel's toolbar is untouched");
        assert!(app.ui.toolbar_edits.basic.is_none());
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Nope"})).is_err());
        let r = app.run("toolbar.removeTool", json!({"tool": "artboard", "toolbar": "Mine"})).unwrap();
        assert_eq!(r, json!([["pen", "curvature"]]));
        let mine = find(&app.ui, "Mine").cloned().unwrap();
        // Another workspace has its own toolbars (none); coming back brings Mine back.
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(app.ui.custom_toolbars.is_empty());
        assert!(toolbars_menu(&app).iter().all(|(l, ..)| *l != "Mine"));
        app.run("window.workspace", json!({"name": "Drawing"})).unwrap();
        assert_eq!(find(&app.ui, "Mine"), Some(&mine));
        // Saved under a workspace and in the preferences, it loads again.
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        let mut again = reloaded(&app);
        assert_eq!(find(&again.ui, "Mine"), Some(&mine));
        crate::workspaces::switch(&mut again, "Essentials").unwrap();
        crate::workspaces::switch(&mut again, "Drawing").unwrap();
        assert_eq!(find(&again.ui, "Mine"), Some(&mine));
        // Reset makes it blank again.
        assert_eq!(app.run("toolbar.reset", json!({"toolbar": "Mine"})).unwrap(), json!([]));
        // Workspaces saved before New Toolbar load without toolbars.
        let mut old = serde_json::to_value(crate::workspaces::WorkspaceRecord::default()).unwrap();
        old.as_object_mut().unwrap().remove("toolbars");
        assert!(serde_json::from_value::<crate::workspaces::WorkspaceRecord>(old).unwrap().toolbars.is_empty());
    }

    /// What hides a toolbar: Window › Tools hides the Tools panel alone and a toolbar still shows, with
    /// the drawer open for it; a strip torn off a toolbar follows the toolbar and one torn off Tools
    /// follows Tools; Tab hides the toolbar and a second Tab shows it; making, copying or showing a
    /// toolbar while Tab hides the panels shows them again, with the other toolbars as they were
    /// (VectorCraft's own rule: showing one does not turn the others off).
    #[test]
    fn a_new_toolbar_shows_without_the_tools_panel_until_tab() {
        let mut app = app();
        let ctx = crate::testing::ui_context();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "curvature", "toolbar": "Mine", "slot": 0, "place": "group"})).unwrap();
        app.run("window.floatTools", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        app.run("window.floatTools", json!({"tool": "pen"})).unwrap();
        window_frame(&mut app, &ctx, 0.0, 800.0);
        window_frame(&mut app, &ctx, 0.1, 800.0);
        let mine = panel_id(find(&app.ui, "Mine").unwrap().id);
        let (mine_strip, tools_strip) = (strip_id(&Owner::Toolbar(find(&app.ui, "Mine").unwrap().id), "pen"), strip_id(&Owner::Tools, "pen"));
        assert!(panel_docking::visible(&app, &mine) && panel_docking::visible(&app, &mine_strip) && panel_docking::visible(&app, &tools_strip));
        // Window › Tools hides the Tools panel and its strip alone.
        app.run("window.toolbar", json!({})).unwrap();
        window_frame(&mut app, &ctx, 0.2, 800.0);
        assert!(!crate::toolbar::shown(&app.ui));
        assert!(!panel_docking::visible(&app, &tools_strip), "a strip torn off Tools stays hidden with Tools");
        assert!(panel_docking::visible(&app, &mine) && panel_docking::visible(&app, &mine_strip));
        app.run("window.editToolbar", json!({"toolbar": "Mine", "open": true})).unwrap();
        assert_eq!(drawer_target(&app.ui), Some("Mine"));
        // Hiding the toolbar takes its strip with it.
        app.run("window.toolbar.custom", json!({"name": "Mine", "show": false})).unwrap();
        assert!(!panel_docking::visible(&app, &mine_strip), "a strip follows its toolbar");
        assert_eq!(drawer_target(&app.ui), None, "the drawer beside a hidden toolbar closes");
        app.run("window.toolbar.custom", json!({"name": "Mine", "show": true})).unwrap();
        // Tab hides it and a second Tab shows it.
        tab(&mut app, &ctx, 1.0);
        assert!(!panel_docking::panels_shown(&app));
        tab(&mut app, &ctx, 2.0);
        assert!(panel_docking::panels_shown(&app) && shown(&app, "Mine"));
        // Made, copied or shown while Tab hides the panels, a toolbar ends the hide.
        for (k, (command, params)) in [
            ("window.toolbar.new", json!({"name": "Other"})),
            ("window.toolbar.duplicate", json!({"name": "Mine"})),
            ("window.toolbar.custom", json!({"name": "Other", "show": true})),
        ]
        .into_iter()
        .enumerate()
        {
            tab(&mut app, &ctx, 3.0 + k as f64);
            assert!(!panel_docking::panels_shown(&app), "{command}: hidden by Tab");
            app.run(command, params).unwrap();
            assert!(panel_docking::panels_shown(&app), "{command} ends the hide");
            assert!(shown(&app, "Mine"), "{command}: the other toolbars stay on");
        }
    }

    /// A strip torn off a toolbar goes back when the group it was torn from goes.
    #[test]
    fn groups_torn_off_follow_the_toolbars_tools() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        for tool in ["curvature", "rectangle"] {
            app.run("toolbar.addTool", json!({"tool": tool, "toolbar": "Mine", "slot": 0, "place": "group"})).unwrap();
        }
        app.run("window.floatTools", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        assert!(crate::toolbar::is_floating(&app, Some("Mine"), "pen"));
        app.run("toolbar.removeTool", json!({"tool": "curvature", "toolbar": "Mine"})).unwrap();
        assert!(crate::toolbar::is_floating(&app, Some("Mine"), "pen"), "the group is still there");
        app.run("toolbar.reset", json!({"toolbar": "Mine"})).unwrap();
        assert!(!crate::toolbar::is_floating(&app, Some("Mine"), "pen"));
    }

    /// The touch preference switches to the Touch workspace the way choosing the Touch
    /// Workspace does: the toolbars made in the workspace left come back with it.
    #[test]
    fn the_touch_workspace_brings_the_toolbars_back() {
        let mut app = app();
        app.run("window.workspace", json!({"name": "Typography"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.session.prefs.touch_workspace = true;
        crate::workspaces::sync_touch_workspace(&mut app, true);
        assert!(crate::workspaces::is_current(&app.ui, crate::workspaces::TOUCH));
        assert!(app.ui.custom_toolbars.is_empty());
        app.session.prefs.touch_workspace = false;
        crate::workspaces::sync_touch_workspace(&mut app, true);
        assert!(crate::workspaces::is_current(&app.ui, "Typography"));
        assert!(find(&app.ui, "Mine").is_some(), "back with Typography");
    }

    /// Reset Workspace keeps the toolbars made in the workspace, in a built-in one too, and New
    /// Workspace leaves the workspace it starts from with its toolbars: the new one starts with a
    /// copy, and switching back brings the first workspace's own back.
    #[test]
    fn toolbars_stay_in_their_workspace_through_reset_and_new_workspace() {
        let mut app = app();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        app.run("window.toolbar", json!({})).unwrap();
        app.run("window.workspace.reset", json!({})).unwrap();
        assert!(crate::toolbar::shown(&app.ui), "the layout is reset");
        assert_eq!(find(&app.ui, "Mine").map(|b| b.slots.clone()), Some(vec![vec!["pen".to_string()]]), "Mine stays in Essentials");
        let mine = find(&app.ui, "Mine").cloned().unwrap();
        // The layout changes go into the new workspace; Essentials keeps only its toolbars.
        app.run("window.toolbar", json!({})).unwrap();
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        assert_eq!(find(&app.ui, "Mine"), Some(&mine), "Drawing starts with a copy");
        app.run("window.toolbar.new", json!({"name": "Only in Drawing"})).unwrap();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(crate::toolbar::shown(&app.ui), "Essentials as saved");
        assert_eq!(app.ui.custom_toolbars, std::slice::from_ref(&mine), "with its own toolbars");
        app.run("window.workspace", json!({"name": "Drawing"})).unwrap();
        assert_eq!(app.ui.custom_toolbars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Mine", "Only in Drawing"]);
    }

    fn listed(app: &VectorcraftApp) -> Vec<String> {
        toolbars_menu(app).into_iter().filter(|(_, id, _)| *id == "window.toolbar.custom").map(|(l, ..)| l).collect()
    }

    /// Manage Toolbars renames, copies and deletes the toolbars made with New Toolbar: each change
    /// shows under Window › Toolbars and stays in the workspace after switching away and back.
    #[test]
    fn manage_toolbars_renames_duplicates_and_deletes() {
        let mut app = app();
        let menu = toolbars_menu(&app);
        assert_eq!(menu.last().map(|(l, id, _)| (l.as_str(), id.as_str())), Some(("Manage Toolbars…", "window.toolbar.manage")));
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        let back = |app: &mut VectorcraftApp| {
            app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
            app.run("window.workspace", json!({"name": "Drawing"})).unwrap();
        };
        // Rename.
        assert_eq!(app.run("window.toolbar.rename", json!({"name": "mine", "to": " Pens "})).unwrap(), json!({"toolbar": "Pens"}));
        assert_eq!(listed(&app), ["Pens", "Other"]);
        back(&mut app);
        assert_eq!(listed(&app), ["Pens", "Other"]);
        assert_eq!(find(&app.ui, "Pens").unwrap().slots, [["pen"]]);
        assert!(app.run("window.toolbar.rename", json!({"name": "Pens", "to": "other"})).unwrap_err().contains("already"));
        assert!(app.run("window.toolbar.rename", json!({"name": "Pens", "to": " "})).unwrap_err().contains("empty"));
        assert!(app.run("window.toolbar.rename", json!({"name": "Nope", "to": "X"})).unwrap_err().contains("Nope"));
        // Only the case changes.
        app.run("window.toolbar.rename", json!({"name": "Pens", "to": "PENS"})).unwrap();
        app.run("window.toolbar.rename", json!({"name": "pens", "to": "Pens"})).unwrap();
        // Duplicate: a copy with its tools, shown; without a name, a blank toolbar.
        app.run("window.toolbar.custom", json!({"name": "Pens", "show": false})).unwrap();
        assert_eq!(app.run("window.toolbar.duplicate", json!({"name": "pens"})).unwrap(), json!({"toolbar": "Pens Copy"}));
        assert_eq!(app.run("window.toolbar.duplicate", json!({"name": "Pens"})).unwrap(), json!({"toolbar": "Pens Copy 2"}));
        assert_eq!(app.run("window.toolbar.duplicate", json!({})).unwrap(), json!({"toolbar": "Toolbar 1"}));
        let copy = find(&app.ui, "Pens Copy").unwrap();
        assert!(shown(&app, "Pens Copy") && copy.slots == [["pen"]]);
        let layout = panel_docking::layout_of(&app.ui);
        let window = |name: &str| panel_docking::floating_window(layout, &panel_id(find(&app.ui, name).unwrap().id)).map(|w| w.rect.min);
        assert!(window("Pens Copy").is_some(), "a copy floats");
        assert_ne!(window("Pens Copy"), window("Pens"), "not on top of the toolbar it copies");
        assert_ne!(window("Pens Copy"), window("Pens Copy 2"), "nor on top of an earlier copy");
        assert!(find(&app.ui, "Toolbar 1").unwrap().slots.is_empty());
        assert_eq!(listed(&app), ["Pens", "Other", "Pens Copy", "Pens Copy 2", "Toolbar 1"]);
        // The copy is a toolbar of its own.
        app.run("toolbar.addTool", json!({"tool": "rectangle", "toolbar": "Pens Copy"})).unwrap();
        assert_eq!(find(&app.ui, "Pens").unwrap().slots, [["pen"]]);
        back(&mut app);
        assert_eq!(listed(&app), ["Pens", "Other", "Pens Copy", "Pens Copy 2", "Toolbar 1"]);
        assert!(app.run("window.toolbar.duplicate", json!({"name": "Nope"})).is_err());
        // Delete.
        app.run("window.toolbar.delete", json!({"name": "pens copy 2"})).unwrap();
        app.run("window.toolbar.delete", json!({"name": "Toolbar 1"})).unwrap();
        assert_eq!(listed(&app), ["Pens", "Other", "Pens Copy"]);
        back(&mut app);
        assert_eq!(listed(&app), ["Pens", "Other", "Pens Copy"]);
        assert!(app.run("window.toolbar.delete", json!({"name": "Toolbar 1"})).unwrap_err().contains("Toolbar 1"));
        // Kept in the saved workspace and the preferences.
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        let mut again = reloaded(&app);
        crate::workspaces::switch(&mut again, "Essentials").unwrap();
        crate::workspaces::switch(&mut again, "Drawing").unwrap();
        assert_eq!(again.ui.custom_toolbars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Pens", "Other", "Pens Copy"]);
    }

    /// Renaming or deleting the toolbar the All Tools drawer edits closes the drawer; a new name
    /// that changes only the case leaves it open.
    #[test]
    fn renaming_or_deleting_the_toolbar_being_edited_closes_the_drawer() {
        let mut app = app();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        app.run("window.editToolbar", json!({"toolbar": "Mine", "open": true})).unwrap();
        app.run("window.toolbar.rename", json!({"name": "Other", "to": "Second"})).unwrap();
        assert_eq!(drawer_target(&app.ui), Some("Mine"), "another toolbar's rename leaves it open");
        app.run("window.toolbar.rename", json!({"name": "Mine", "to": "MINE"})).unwrap();
        assert_eq!(drawer_target(&app.ui), Some("MINE"), "a change of case leaves it open");
        app.run("window.toolbar.rename", json!({"name": "MINE", "to": "Mine"})).unwrap();
        app.run("window.toolbar.rename", json!({"name": "Mine", "to": "Pens"})).unwrap();
        assert_eq!(drawer_target(&app.ui), None);
        app.run("window.editToolbar", json!({"toolbar": "Pens", "open": true})).unwrap();
        app.run("window.toolbar.delete", json!({"name": "Pens"})).unwrap();
        assert!(!app.ui.tools_drawer && app.ui.drawer_toolbar.is_none());
    }

    /// A renamed or copied toolbar's name keeps within [`MAX_NAME`] characters, so it is not dropped
    /// when the preferences load again.
    #[test]
    fn renamed_and_copied_toolbars_keep_names_short_enough_to_load() {
        let mut app = app();
        let long = "x".repeat(MAX_NAME);
        app.run("window.toolbar.new", json!({"name": long})).unwrap();
        assert!(app.run("window.toolbar.rename", json!({"name": long, "to": "y".repeat(MAX_NAME + 1)})).unwrap_err().contains("at most"));
        let copy = app.run("window.toolbar.duplicate", json!({"name": long})).unwrap();
        let copy = copy["toolbar"].as_str().unwrap().to_string();
        assert!(copy.ends_with(" Copy") && copy.chars().count() <= MAX_NAME, "{copy}");
        let second = app.run("window.toolbar.duplicate", json!({"name": long})).unwrap();
        assert!(second["toolbar"].as_str().unwrap().ends_with(" Copy 2"));
        let mut bars = app.ui.custom_toolbars.clone();
        sanitize(&mut bars);
        assert_eq!(bars.len(), 3);
    }

    /// A hand-edited file: unnamed toolbars and repeated names go, unknown tools go, each tool once,
    /// a group shows a tool chosen last only when the group holds it, ids are unique and the list is
    /// cut at [`MAX`].
    #[test]
    fn toolbars_load_sanitized() {
        let bar = |id: u32, name: &str, slots: &[&[&str]]| CustomToolbar {
            slots: slots.iter().map(|s| s.iter().map(|id| id.to_string()).collect()).collect(),
            ..blank(id, name.into())
        };
        let slot = crate::toolbar::slot_id;
        let long = "x".repeat(MAX_NAME + 1);
        let mut bars = vec![
            bar(0, "A", &[&["pen", "nope", "curvature"], &["pen"], &["nope"]]),
            bar(3, "a", &[]),
            bar(3, " ", &[]),
            bar(3, &long, &[]),
            bar(3, "B", &[]),
            bar(3, "C", &[&["pen"]]),
        ];
        // The tool a group shows: kept only for a group the toolbar holds, and a tool in it.
        bars[0].last_used.set(slot("pen"), "curvature".into());
        bars[0].last_used.set(slot("rectangle"), "pen".into());
        bars[4].last_used.set(slot("pen"), "curvature".into());
        bars[5].last_used.set(slot("pen"), "curvature".into());
        sanitize(&mut bars);
        assert_eq!(bars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["A", "B", "C"], "a name past {MAX_NAME} characters goes too");
        assert_eq!(bars.iter().map(|b| b.id).collect::<Vec<_>>(), [1, 3, 2], "the first has none, the third repeats the second's");
        assert_eq!(bars[0].last_used.get(slot("pen")), Some(&"curvature".to_string()));
        assert!(bars[0].last_used.get(slot("rectangle")).is_none(), "the toolbar holds no such group");
        assert!(bars[1].last_used.get(slot("pen")).is_none() && bars[2].last_used.get(slot("pen")).is_none());
        assert_eq!(bars[0].slots, [["pen", "curvature"]]);
        let mut many: Vec<CustomToolbar> = (0..MAX + 3).map(|i| bar(0, &format!("T{i}"), &[])).collect();
        sanitize(&mut many);
        assert_eq!(many.len(), MAX);
        assert_eq!(many.iter().map(|b| b.id).collect::<BTreeSet<_>>().len(), MAX, "every toolbar has an id of its own");
    }
}
