//! Window › Toolbars › New Toolbar (#1759): toolbars the user makes and names. New Toolbar asks
//! for a name and makes a blank toolbar that floats on its own; the ••• button at its foot opens the
//! All Tools drawer beside it, and tools dragged from the drawer fill it (`edit_toolbar`, with the
//! `toolbar` param of its commands). The + button above it opens Add Tool…, a menu of every tool
//! where a click adds the tool at the end, and whose Remove Tool items take tools out: VectorCraft's
//! own way to fill a toolbar without a drag. Its title bar moves it, its × hides it and its double
//! arrow shows its tools in one column or two. Each toolbar is listed under Window › Toolbars, where its
//! item shows or hides it, and is kept with the workspace it was made in (`Workspace::toolbars`).
//! Window › Toolbars › Manage Toolbars (also in the drawer's menu) renames, copies and deletes them.
//!
//! A toolbar docks in a lane along the window's left or right edge (on the left, right of the Tools
//! panel docked there or beside the canvas), when its title bar is dropped on the edge (lit up while it is dragged), or with
//! Dock to Left and Dock to Right in its title bar's context menu (`window.toolbar.dock`). Several
//! toolbars docked at one edge stack from the top in the order they docked, each with its share of
//! the lane's height, and the lane is as wide as its widest toolbar (a lane too short for its
//! stack wraps it into more columns, up to half the width beside it, and past that the toolbars
//! show only their title bars, which the mouse wheel scrolls through when even they do not fit, so
//! none covers another). Dragging a docked toolbar's
//! title bar out of it floats it under the pointer, as Float Toolbar does where it floated last
//! (`window.toolbar.float`); Escape during the drag puts it back. These are VectorCraft's own rules.
//!
//! Behavior from the public documentation:
//! <https://helpx.adobe.com/illustrator/using/tools.html> (Create and manage toolbars: New Toolbar
//! from Window › Toolbars or the drawer's flyout menu, a name and OK make a blank toolbar, its Edit
//! Toolbar button opens the drawer to add tools, and a toolbar stays in the workspace it is made in;
//! a toolbar moves by its title bar, whose double arrow toggles one column or two and whose close
//! button hides it. Manage toolbars: Manage Toolbars from Window › Toolbars or the drawer's flyout
//! menu; a new name typed for the selected toolbar renames it, + copies the selected toolbar or,
//! with none selected, makes a new one, and Delete deletes it).

use std::sync::atomic::Ordering;

use egui::{CornerRadius, Sense, Stroke, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_tools::tool_info;

use crate::state::{CustomToolbar, Dialog, UiState};
use crate::theme::Tokens;
use crate::{VectorcraftApp, icons, widgets};

/// The most toolbars a workspace keeps.
pub(crate) const MAX: usize = 64;
/// The most characters a toolbar's name has.
const MAX_NAME: usize = 200;
/// Width of a toolbar of one column of tool buttons, inside its border.
const WIDTH: f32 = 44.0;
/// Width a second column of tool buttons adds.
const BUTTON: f32 = 36.0;
/// Height of the title bar at a toolbar's top, which moves it.
const GRIP: f32 = 14.0;
/// Width of the × and the double arrow at the ends of the title bar.
const TITLE_BUTTON: f32 = 12.0;
/// Where the first toolbar made floats, right of the Tools panel; the next ones go lower.
const NEW_AT: [f32; 2] = [96.0, 120.0];
/// The space around each toolbar docked in a lane, and between two of them.
const LANE_GAP: f32 = 6.0;
/// Width of the strip along the window's edge where a dropped title bar docks its toolbar, while
/// that edge's lane is narrower or empty.
const ZONE: f32 = 48.0;
/// Points the pointer travels from its press before a drop docks (a click on the title bar of a
/// toolbar floating near the edge doesn't dock it).
const TRAVEL: f32 = 4.0;

/// A toolbar's name without leading and trailing spaces, in lowercase. Names with the same key are
/// the same name: lookups, the check that a name is free, sanitizing and each toolbar's area
/// compare keys.
pub(crate) fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The area a toolbar is drawn in.
pub(crate) fn area_id(name: &str) -> egui::Id {
    egui::Id::unique(("custom-toolbar", key(name)))
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

/// Make a blank toolbar named `name`, shown; returns its name.
pub(crate) fn create(ui: &mut UiState, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("toolbar name can't be empty".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("a toolbar name has at most {MAX_NAME} characters"));
    }
    if find(ui, name).is_some() {
        return Err(format!("there is already a toolbar named `{name}`"));
    }
    if ui.custom_toolbars.len() >= MAX {
        return Err(format!("a workspace keeps at most {MAX} toolbars"));
    }
    let n = ui.custom_toolbars.len() as f32;
    let pos = [NEW_AT[0] + 16.0 * n, NEW_AT[1] + 24.0 * n];
    ui.custom_toolbars.push(CustomToolbar {
        name: name.to_string(),
        slots: vec![],
        shown: true,
        pos,
        dock: None,
        dock_right: false,
        slot_tool: Default::default(),
        double: false,
        floating: vec![],
    });
    show_alone_after_tab(ui, name);
    Ok(name.to_string())
}

/// Number the toolbars docked at each edge 0, 1, 2… from the top, in their order (in the list's
/// order where two have the same place).
fn renumber(bars: &mut [CustomToolbar]) {
    for right in [false, true] {
        let mut on: Vec<(Option<u32>, usize)> =
            bars.iter().enumerate().filter(|(_, b)| b.dock.is_some() && b.dock_right == right).map(|(k, b)| (b.dock, k)).collect();
        on.sort_unstable();
        for (n, (_, k)) in on.into_iter().enumerate() {
            if let Some(b) = bars.get_mut(k) {
                b.dock = Some(n as u32);
            }
        }
    }
}

/// Dock the toolbar at `k` at the right edge or the left, at place `at` of that edge's stack (none
/// or past the end: at the end); the toolbars from that place down move down one. Returns its place.
fn dock_at(bars: &mut [CustomToolbar], k: usize, right: bool, at: Option<u32>) -> u32 {
    if let Some(b) = bars.get_mut(k) {
        b.dock = None;
    }
    renumber(bars);
    let n = bars.iter().filter(|b| b.dock.is_some() && b.dock_right == right).count() as u32;
    let at = at.map_or(n, |at| at.min(n));
    for b in bars.iter_mut().filter(|b| b.dock_right == right) {
        if let Some(o) = b.dock.as_mut()
            && *o >= at
        {
            *o += 1;
        }
    }
    if let Some(b) = bars.get_mut(k) {
        b.dock = Some(at);
        b.dock_right = right;
    }
    at
}

/// Where the toolbar is: `{toolbar, floating, side, order, pos}`, the edge it docks at (while it
/// floats, the edge it docked at last), its place in that edge's stack while docked (else null) and
/// where it floats (while docked, where it floated last).
fn placement(bar: &CustomToolbar) -> Value {
    let side = if bar.dock_right { "right" } else { "left" };
    json!({ "toolbar": bar.name, "floating": bar.dock.is_none(), "side": side, "order": bar.dock, "pos": bar.pos })
}

/// The toolbars that show docked at the right edge or the left, by index, from the top.
fn docked(ui: &UiState, right: bool) -> Vec<usize> {
    let mut on: Vec<(Option<u32>, usize)> = ui
        .custom_toolbars
        .iter()
        .enumerate()
        .filter(|(_, b)| b.shown && b.dock.is_some() && b.dock_right == right)
        .map(|(k, b)| (b.dock, k))
        .collect();
    on.sort_unstable();
    on.into_iter().map(|(_, k)| k).collect()
}

/// Width of `bar` inside its border: one column of tool buttons, or two.
fn bar_width(bar: &CustomToolbar) -> f32 {
    WIDTH + if bar.double { BUTTON } else { 0.0 }
}

/// The least height a docked toolbar draws at: its border, its title bar and the gap under it, the
/// least height its tools scroll in (`egui::ScrollArea`'s default `min_scrolled_height`, 64 points)
/// and its bottom margin.
const DOCKED_LEAST: f32 = 2.0 + GRIP + 4.0 + 64.0 + 4.0;

/// The columns a lane `height` points tall lays `n` docked toolbars out in, at most `most` of them,
/// and the most toolbars each column holds: one column while each toolbar gets at least
/// [`DOCKED_LEAST`] points of the lane's height, else as few more columns as give every toolbar
/// that much, so none covers another. With `most` columns too few for that, the toolbars spread
/// evenly over `most` columns, and those with less than [`DOCKED_LEAST`] points show only their
/// title bars ([`docked_places`]).
fn lane_columns(n: usize, height: f32, most: usize) -> (usize, usize) {
    let fit = ((height - LANE_GAP) / (DOCKED_LEAST + LANE_GAP)).floor();
    let fit = if fit.is_finite() && fit >= 1.0 { fit as usize } else { 1 };
    let per = n.div_ceil(n.div_ceil(fit).clamp(1, most.max(1))).max(1);
    (n.div_ceil(per).max(1), per)
}

/// The most columns of toolbars at most `widest` points wide inside their borders that `width`
/// points hold with [`LANE_GAP`] around each: at least one.
fn most_columns(width: f32, widest: f32) -> usize {
    let most = ((width - LANE_GAP) / (widest.max(1.0) + LANE_GAP) + 1e-3).floor();
    if most.is_finite() && most >= 1.0 { most as usize } else { 1 }
}

/// Toolbar `name`, shown after Tab hid the toolbars, shows alone, as Window › Tools shows the
/// Tools panel: the other toolbars Tab hid are turned off, and the next Tab hides it.
fn show_alone_after_tab(ui: &mut UiState, name: &str) {
    if ui.tab_hid.take().is_some() {
        for b in ui.custom_toolbars.iter_mut().filter(|b| b.name != name) {
            b.shown = false;
        }
    }
}

/// Where the toolbar named `name` is in the list, whatever its case.
fn index(ui: &UiState, name: &str) -> Result<usize, String> {
    let k = key(name);
    ui.custom_toolbars.iter().position(|b| key(&b.name) == k).ok_or_else(|| format!("no toolbar named `{name}`"))
}

/// The All Tools drawer beside the toolbar named `old`, and a group's flyout open on it (`ctx`),
/// close when it is deleted (`new` is `None`) or renamed; a new name that changes only the case
/// leaves them open on it. A drag of its title bar in progress ends with them, so Escape does not
/// put a toolbar that takes the name later back where this one was.
fn close_popups(ui: &mut UiState, ctx: Option<&egui::Context>, old: &str, new: Option<&str>) {
    let same = new.is_some_and(|n| key(n) == key(old));
    if drawer_target(ui).map(key) == Some(key(old)) {
        if same {
            ui.drawer_toolbar = new.map(String::from);
        } else {
            ui.tools_drawer = false;
            ui.drawer_toolbar = None;
        }
    }
    let Some(ctx) = ctx else { return };
    if !same && ctx.data(|d| d.get_temp::<Move>(move_id())).is_some_and(|m| key(&m.name) == key(old)) {
        ctx.data_mut(|d| d.remove::<Move>(move_id()));
    }
    let id = crate::toolbar::flyout_toolbar_id();
    if ui.flyout.is_some() && ctx.data(|d| d.get_temp::<Option<String>>(id)).flatten().is_some_and(|t| key(&t) == key(old)) {
        if same {
            ctx.data_mut(|d| d.insert_temp(id, new.map(String::from)));
        } else {
            ui.flyout = None;
        }
    }
}

/// Give the toolbar named `name` the name `to`; returns the new name.
pub(crate) fn rename(ui: &mut UiState, ctx: Option<&egui::Context>, name: &str, to: &str) -> Result<String, String> {
    let to = to.trim();
    if to.is_empty() {
        return Err("toolbar name can't be empty".into());
    }
    if to.chars().count() > MAX_NAME {
        return Err(format!("a toolbar name has at most {MAX_NAME} characters"));
    }
    let k = index(ui, name)?;
    if ui.custom_toolbars.iter().enumerate().any(|(i, b)| i != k && key(&b.name) == key(to)) {
        return Err(format!("there is already a toolbar named `{to}`"));
    }
    let old = ui.custom_toolbars.get(k).map(|b| b.name.clone()).unwrap_or_default();
    close_popups(ui, ctx, &old, Some(to));
    if let Some(b) = ui.custom_toolbars.get_mut(k) {
        b.name = to.to_string();
    }
    Ok(to.to_string())
}

/// A copy of the toolbar named `name`, with its tools, shown a little below it (further down
/// while an earlier copy is there) and called
/// "`name` Copy" (then "`name` Copy 2"…); without `name`, a blank toolbar called "Toolbar N".
/// Returns the new toolbar's name.
pub(crate) fn duplicate(ui: &mut UiState, name: Option<&str>) -> Result<String, String> {
    let Some(name) = name else {
        let name = (1..).map(|i| format!("Toolbar {i}")).find(|n| find(ui, n).is_none()).unwrap_or_default();
        return create(ui, &name);
    };
    let k = index(ui, name)?;
    if ui.custom_toolbars.len() >= MAX {
        return Err(format!("a workspace keeps at most {MAX} toolbars"));
    }
    let Some(mut copy) = ui.custom_toolbars.get(k).cloned() else { return Err(format!("no toolbar named `{name}`")) };
    // A long name is shortened so that "Copy" and its number fit in MAX_NAME characters.
    let stem: String = copy.name.chars().take(MAX_NAME - " Copy 64".len()).collect();
    let base = format!("{} Copy", stem.trim_end());
    copy.name = std::iter::once(base.clone()).chain((2..).map(|i| format!("{base} {i}"))).find(|n| find(ui, n).is_none()).unwrap_or_default();
    copy.shown = true;
    // A copy of a docked toolbar floats, below where that toolbar floated last.
    copy.dock = None;
    let from = copy.pos;
    let at = |n: f32| [from[0] + 16.0 * n, from[1] + 24.0 * n];
    let n = (1..=MAX + 1).map(|n| n as f32).find(|&n| ui.custom_toolbars.iter().all(|b| b.pos != at(n))).unwrap_or(1.0);
    copy.pos = at(n);
    for f in &mut copy.floating {
        f.pos = [f.pos[0] + 16.0 * n, f.pos[1] + 24.0 * n];
    }
    let name = copy.name.clone();
    ui.custom_toolbars.push(copy);
    show_alone_after_tab(ui, &name);
    Ok(name)
}

/// Delete the toolbar named `name`, with the groups torn off it.
pub(crate) fn delete(ui: &mut UiState, ctx: Option<&egui::Context>, name: &str) -> Result<(), String> {
    let k = index(ui, name)?;
    close_popups(ui, ctx, name, None);
    ui.custom_toolbars.remove(k);
    renumber(&mut ui.custom_toolbars);
    Ok(())
}

/// Keep named toolbars, each name once whatever its case and at most [`MAX_NAME`] characters, of
/// known tools, each once in its toolbar, at a position that is a number, with one strip per group
/// torn off them, each group's tool chosen last only when the group holds it, and the docked ones
/// numbered 0, 1, 2… at each edge (a hand-edited preferences file or workspace).
pub(crate) fn sanitize(bars: &mut Vec<CustomToolbar>) {
    let mut names = std::collections::BTreeSet::new();
    bars.retain_mut(|b| {
        b.name = b.name.trim().to_string();
        let mut seen = std::collections::BTreeSet::new();
        for s in &mut b.slots {
            s.retain(|id| tool_info(id).is_some() && seen.insert(id.clone()));
        }
        b.slots.retain(|s| !s.is_empty());
        let slots = &b.slots;
        b.slot_tool.retain(|first, id| slots.iter().any(|s| s.first() == Some(first) && s.contains(id)));
        crate::state::FloatingFlyout::sanitize(&mut b.floating);
        keep_strips(b);
        if !b.pos.iter().all(|v| v.is_finite()) {
            b.pos = NEW_AT;
        }
        !b.name.is_empty() && b.name.chars().count() <= MAX_NAME && names.insert(key(&b.name))
    });
    bars.truncate(MAX);
    renumber(bars);
}

/// Keep the strips torn off `bar` whose group it still holds, each with the group's tools as they
/// are now; a strip whose group is gone, now starts with another tool or holds one tool alone goes
/// back.
pub(crate) fn keep_strips(bar: &mut CustomToolbar) {
    let slots = &bar.slots;
    bar.floating.retain_mut(|f| match f.tools.first().and_then(|k| slots.iter().find(|s| s.len() > 1 && s.first() == Some(k))) {
        Some(slot) => {
            f.tools.clone_from(slot);
            true
        }
        None => false,
    });
}

/// The commands of the toolbars made with New Toolbar; `None` for other ids.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "window.toolbar.new" => match p.get("name").filter(|v| !v.is_null()) {
            Some(name) => match name.as_str() {
                Some(name) => create(&mut app.ui, name).map(|name| json!({ "toolbar": name })),
                None => Err("name (a toolbar's name) must be a string".into()),
            },
            None => {
                let name = (1..).map(|i| format!("Toolbar {i}")).find(|n| find(&app.ui, n).is_none()).unwrap_or_default();
                app.ui.dialog = Some(Dialog::new(KIND, json!({ "name": name })));
                Ok(Value::Null)
            }
        },
        "window.toolbar.custom" => (|| {
            let show = crate::menus::opt_bool(p, "show")?;
            let name = p.get("name").and_then(Value::as_str).ok_or("name (a toolbar's name) is required")?;
            let bar = find_mut(&mut app.ui, name).ok_or_else(|| format!("no toolbar named `{name}`"))?;
            bar.shown = show.unwrap_or(!bar.shown);
            let (shown, name) = (bar.shown, bar.name.clone());
            if shown {
                show_alone_after_tab(&mut app.ui, &name);
            }
            // The drawer beside a toolbar that is hidden closes with it, and so does an open flyout.
            if !shown {
                app.ui.flyout = None;
                if drawer_target(&app.ui) == Some(name.as_str()) {
                    app.ui.tools_drawer = false;
                    app.ui.drawer_toolbar = None;
                }
            }
            Ok(json!(shown))
        })(),
        "window.toolbar.manage" => {
            app.ui.dialog = Some(Dialog::new(MANAGE, json!({ "selected": "", "name": "" })));
            Ok(Value::Null)
        }
        "window.toolbar.rename" => (|| {
            let name = p.get("name").and_then(Value::as_str).ok_or("name (a toolbar's name) is required")?;
            let to = p.get("to").and_then(Value::as_str).ok_or("to (the new name) is required")?;
            rename(&mut app.ui, app.ui_ctx.as_ref(), name, to).map(|name| json!({ "toolbar": name }))
        })(),
        "window.toolbar.duplicate" => (|| {
            let name = match p.get("name").filter(|v| !v.is_null()) {
                Some(v) => Some(v.as_str().ok_or("name must be a toolbar's name")?),
                None => None,
            };
            duplicate(&mut app.ui, name).map(|name| json!({ "toolbar": name }))
        })(),
        "window.toolbar.delete" => (|| {
            let name = p.get("name").and_then(Value::as_str).ok_or("name (a toolbar's name) is required")?;
            delete(&mut app.ui, app.ui_ctx.as_ref(), name).map(|_| Value::Null)
        })(),
        "window.toolbar.dock" => (|| {
            let name = p.get("name").and_then(Value::as_str).ok_or("name (a toolbar's name) is required")?;
            let k = index(&app.ui, name)?;
            let bar = app.ui.custom_toolbars.get(k).ok_or_else(|| format!("no toolbar named `{name}`"))?;
            // Docking: at the edge it docked at last unless `side` says which.
            let right = match p.get("side") {
                None | Some(Value::Null) => bar.dock_right,
                // `Start` and `End`, as panels take them, too.
                Some(Value::String(s)) if s.eq_ignore_ascii_case("left") || s.eq_ignore_ascii_case("start") => false,
                Some(Value::String(s)) if s.eq_ignore_ascii_case("right") || s.eq_ignore_ascii_case("end") => true,
                Some(Value::String(s)) => return Err(format!("side is left or right, not `{s}`")),
                Some(v) => return Err(format!("side is left or right, not `{v}`")),
            };
            let others = app.ui.custom_toolbars.iter().enumerate().filter(|&(i, b)| i != k && b.dock.is_some() && b.dock_right == right).count();
            let at = match p.get("order") {
                None | Some(Value::Null) => None,
                Some(v) => match v.as_u64().filter(|&n| n <= others as u64) {
                    Some(n) => Some(n as u32),
                    None => return Err(format!("order is a whole number from 0 to {others}, not `{v}`")),
                },
            };
            // Docked at that edge already, with no place asked for, it stays where it is.
            if !(bar.dock.is_some() && bar.dock_right == right && at.is_none()) {
                dock_at(&mut app.ui.custom_toolbars, k, right, at);
            }
            app.ui.custom_toolbars.get(k).map(placement).ok_or_else(|| format!("no toolbar named `{name}`"))
        })(),
        "window.toolbar.float" => (|| {
            let name = p.get("name").and_then(Value::as_str).ok_or("name (a toolbar's name) is required")?;
            let k = index(&app.ui, name)?;
            let at = crate::tools_float::position(p)?;
            if let Some(b) = app.ui.custom_toolbars.get_mut(k) {
                b.dock = None;
                if let Some(at) = at {
                    b.pos = [at.x, at.y];
                }
            }
            renumber(&mut app.ui.custom_toolbars);
            app.ui.custom_toolbars.get(k).map(placement).ok_or_else(|| format!("no toolbar named `{name}`"))
        })(),
        "window.toolbar.list" => Ok(json!(
            app.ui
                .custom_toolbars
                .iter()
                .map(|b| {
                    let side = if b.dock_right { "right" } else { "left" };
                    json!({"name": b.name, "shown": b.shown, "double": b.double, "slots": b.slots, "floating": b.dock.is_none(), "side": side, "order": b.dock})
                })
                .collect::<Vec<_>>()
        )),
        _ => return None,
    })
}

/// New Toolbar's dialog.
pub(crate) const KIND: &str = "newToolbar";

/// OK in New Toolbar.
pub(crate) fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(d) = app.ui.dialog.clone() else { return Err("no dialog open".into()) };
    let name = create(&mut app.ui, &d.str("name"))?;
    app.ui.dialog = None;
    Ok(json!({ "toolbar": name }))
}

/// The New Toolbar dialog: the name, OK and Cancel.
pub(crate) fn show_dialog(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let (mut ok, mut cancel) = (false, false);
    crate::dialogs::modal::show(ctx, "New Toolbar", egui::Id::unique("dialog-new-toolbar"), -40.0, 22, |ui| {
        ui.set_width(320.0);
        crate::dialogs::modal::heading(ui, tl!("New Toolbar"));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim));
            let mut name = d.str("name");
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(220.0)).changed() {
                d.fields.insert("name".into(), json!(name));
            }
        });
        ui.add_space(16.0);
        // As wide as the field and as tall as the buttons, as in `dialogs::show`: a right-to-left
        // layout would take all the height left in the window and center the buttons in it.
        let row = egui::vec2(ui.min_rect().width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::primary_button(ui, tl!("OK")).clicked() {
                ok = true;
            }
            ui.add_space(8.0);
            if widgets::secondary_button(ui, tl!("Cancel")).clicked() {
                cancel = true;
            }
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        ok = true;
    }
    app.ui.dialog = Some(d);
    if cancel {
        app.ui.dialog = None;
    } else if ok && let Err(e) = confirm(app) {
        app.status(e);
    }
}

/// Manage Toolbars' dialog.
pub(crate) const MANAGE: &str = "manageToolbars";

/// The name typed in Manage Toolbars' text box becomes the selected toolbar's (a no-op while it is
/// the same). Returns true when the selected toolbar was deleted or renamed while the dialog was
/// open (a command, another workspace): none is selected then, and the name typed for it is dropped.
fn rename_selected(app: &mut VectorcraftApp) -> Result<bool, String> {
    let Some(d) = app.ui.dialog.as_ref() else { return Ok(false) };
    let (sel, name) = (d.str("selected"), d.str("name"));
    if !sel.is_empty() && find(&app.ui, &sel).is_none() {
        if let Some(d) = app.ui.dialog.as_mut() {
            d.fields.insert("selected".into(), json!(""));
            d.fields.insert("name".into(), json!(""));
        }
        return Ok(true);
    }
    if sel.is_empty() || sel == name.trim() {
        return Ok(false);
    }
    let new = app.run("window.toolbar.rename", json!({ "name": sel, "to": name }))?;
    let new = new.get("toolbar").and_then(Value::as_str).unwrap_or_default().to_string();
    if let Some(d) = app.ui.dialog.as_mut() {
        d.fields.insert("selected".into(), json!(new));
        d.fields.insert("name".into(), json!(new));
    }
    Ok(false)
}

/// OK in Manage Toolbars: the typed name is kept, and the dialog closes.
pub(crate) fn confirm_manage(app: &mut VectorcraftApp) -> Result<Value, String> {
    rename_selected(app)?;
    app.ui.dialog = None;
    Ok(Value::Null)
}

/// The Manage Toolbars dialog: the toolbars made with New Toolbar; the selected one's name in the
/// text box, where a new name renames it; + copies the selected toolbar (none selected: makes a
/// blank one), and the trash can deletes it.
pub(crate) fn show_manage(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let (mut ok, mut commit) = (false, false);
    // A toolbar clicked in the list (none: the click left none selected), + (true) or the trash
    // can (false): each applied after the name typed for the selected toolbar.
    let mut pick: Option<String> = None;
    let mut action: Option<bool> = None;
    crate::dialogs::modal::show(ctx, "Manage Toolbars", egui::Id::unique("dialog-manage-toolbars"), -40.0, 22, |ui| {
        ui.set_width(320.0);
        crate::dialogs::modal::heading(ui, tl!("Manage Toolbars"));
        ui.add_space(12.0);
        let sel = d.str("selected");
        egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_min_height(140.0);
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for b in &app.ui.custom_toolbars {
                    let on = !sel.is_empty() && key(&sel) == key(&b.name);
                    if ui.selectable_label(on, egui::RichText::new(&b.name).color(t.text)).clicked() {
                        // A click on the selected toolbar leaves none selected.
                        pick = Some(if on { String::new() } else { b.name.clone() });
                    }
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let mut name = d.str("name");
            let r = ui.add_enabled(!sel.is_empty(), egui::TextEdit::singleline(&mut name).desired_width(220.0));
            if r.changed() {
                d.fields.insert("name".into(), json!(name));
            }
            commit = r.lost_focus();
            let tip = if sel.is_empty() { tl!("New Toolbar") } else { tl!("Duplicate Toolbar") };
            if widgets::icon_button(ui, "plus", tip, false, 24.0).clicked() {
                action = Some(true);
            }
            if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Toolbar"), false, !sel.is_empty(), 24.0).clicked() {
                action = Some(false);
            }
        });
        ui.add_space(16.0);
        let row = egui::vec2(ui.min_rect().width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::primary_button(ui, tl!("OK")).clicked() {
                ok = true;
            }
        });
    });
    app.ui.dialog = Some(d);
    if ok {
        if let Err(e) = confirm_manage(app) {
            app.status(e);
        }
        return;
    }
    manage_step(app, commit, pick, action);
}

/// What the Manage Toolbars dialog does after a frame: the name typed for the selected toolbar is
/// applied when the text box loses focus, another toolbar is clicked (`pick`) or + (`Some(true)`)
/// or the trash can (`Some(false)`) is clicked. When that rename fails, the click is not applied,
/// so the name typed can be corrected; when the selected toolbar has gone meanwhile, + and the trash
/// can are not applied either.
fn manage_step(app: &mut VectorcraftApp, commit: bool, pick: Option<String>, mut action: Option<bool>) {
    if commit || pick.is_some() || action.is_some() {
        match rename_selected(app) {
            Err(e) => {
                app.status(e);
                return;
            }
            // The toolbar + or the trash can was meant for is gone.
            Ok(true) => action = None,
            Ok(false) => {}
        }
    }
    let sel = app.ui.dialog.as_ref().map(|d| d.str("selected")).unwrap_or_default();
    let done = match action {
        Some(true) => Some(app.run("window.toolbar.duplicate", if sel.is_empty() { json!({}) } else { json!({ "name": sel }) })),
        Some(false) => Some(app.run("window.toolbar.delete", json!({ "name": sel }))),
        None => None,
    };
    // The new toolbar is selected, ready to be renamed; after Delete, none is.
    let select = match done {
        Some(Ok(r)) => Some(r.get("toolbar").and_then(Value::as_str).unwrap_or_default().to_string()),
        Some(Err(e)) => {
            app.status(e);
            None
        }
        None => pick,
    };
    if let Some(name) = select
        && let Some(d) = app.ui.dialog.as_mut()
    {
        d.fields.insert("selected".into(), json!(name));
        d.fields.insert("name".into(), json!(name));
    }
}

// ---------- the menu ----------

crate::shortcut_editor::ui_mirror!(fn names_store() -> Vec<&'static str>);

/// Mirror the toolbars' names for the (static) menu tree.
pub fn sync(ui: &UiState) {
    let same = names_store()
        .read()
        .map(|v| v.len() == ui.custom_toolbars.len() && v.iter().zip(&ui.custom_toolbars).all(|(a, b)| *a == b.name))
        .unwrap_or(true);
    if !same {
        if let Ok(mut v) = names_store().write() {
            *v = ui.custom_toolbars.iter().map(|b| crate::shortcut_editor::intern(&b.name)).collect();
        }
        crate::shortcut_editor::GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Window › Toolbars: Advanced, Double Column and Keep Tools Beside the Canvas, the toolbars made
/// with New Toolbar (each checked while it shows), New Toolbar… and Manage Toolbars….
pub fn menu_items() -> Vec<crate::menus::Item> {
    use crate::menus::Item;
    let c = |label: &'static str, id: &'static str| Item::Cmd(label, id, Value::Null);
    let mut v = vec![
        c("Advanced", "window.toolbarAdvanced"),
        c("Double Column", "window.toolbarColumns"),
        c("Keep Tools Beside the Canvas", "window.toolsBesideCanvas"),
        Item::Sep,
    ];
    let names = names_store().read().map(|v| v.clone()).unwrap_or_default();
    if !names.is_empty() {
        v.extend(names.into_iter().map(|n| Item::Cmd(n, "window.toolbar.custom", json!({ "name": n }))));
        v.push(Item::Sep);
    }
    v.push(c("New Toolbar…", "window.toolbar.new"));
    v.push(c("Manage Toolbars…", "window.toolbar.manage"));
    v
}

// ---------- the toolbars ----------

/// What a press on a toolbar did, applied once every toolbar is drawn.
enum Action {
    Select(&'static str),
    Options(&'static str),
    /// A group's flyout: its tools, its button, and the toolbar's name.
    Flyout(Vec<&'static str>, egui::Rect, String),
    EditToolbar(String),
    /// A tool picked in Add Tool…'s menu, added at the end of the toolbar named.
    AddTool(String, &'static str),
    /// A tool picked under Remove Tool in Add Tool…'s menu, taken out of the toolbar named.
    RemoveTool(String, &'static str),
    /// The title bar's ×.
    Hide(String),
    /// The title bar's double arrow: two columns (true) or one.
    Columns(String, bool),
    /// Dock to Left (false) or Dock to Right (true) in the title bar's context menu.
    Dock(String, bool),
    /// Float Toolbar in the title bar's context menu.
    Float(String),
}

/// The lane along the window's right edge (true) or left edge where docked toolbars show, as last
/// laid out; removed while it is empty.
fn lane_id(right: bool) -> egui::Id {
    egui::Id::unique(("custom-toolbar-lane", right))
}

/// Where a title bar dropped docks its toolbar at the right edge (true) or the left, as last laid
/// out.
fn zone_id(right: bool) -> egui::Id {
    egui::Id::unique(("custom-toolbar-zone", right))
}

/// A drag of a toolbar's title bar in progress: the toolbar, where the press began and how far that
/// is from the toolbar's corner, where the toolbar was before the drag (to put it back on Escape)
/// and whether Escape put it back.
#[derive(Clone, Debug)]
struct Move {
    name: String,
    from: egui::Pos2,
    grab: egui::Vec2,
    dock: Option<u32>,
    right: bool,
    pos: [f32; 2],
    cancelled: bool,
}

fn move_id() -> egui::Id {
    egui::Id::unique("custom-toolbar-move")
}

/// The lanes along the window's left and right edges where the docked toolbars that show are drawn,
/// each as wide as its widest toolbar with [`LANE_GAP`] either side (times the columns of
/// [`lane_columns`] when the lane is too short for one, at most half the width left for the lane),
/// and the targets where a title
/// bar dropped docks its toolbar: from the window's edge across the lane, or across a strip
/// [`ZONE`] points wide when the lane is narrower or empty, and on the left across the Tools panel
/// docked there too. Laid out before the canvas and the panels, which take the space left, except
/// the left lane while Tools is docked beside the canvas ([`left_lane_beside_canvas`]). A lane
/// laid out anew draws again, since its toolbars were placed by the lane of the frame before.
pub(crate) fn lanes(app: &VectorcraftApp, ui: &mut egui::Ui) {
    for right in [false, true] {
        if right || !left_lane_beside_canvas(&app.ui) {
            let (edge, room) = (ui.ctx().content_rect(), ui.available_rect_before_wrap().width() / 2.0);
            lane(app, ui, right, edge, room);
        }
    }
}

/// The width the canvas panel keeps for the left lane while it stands beside the canvas
/// ([`left_lane_beside_canvas`]): one column of its widest toolbar, none while no docked toolbar
/// shows there.
pub(crate) fn beside_canvas_reserve(ui: &UiState) -> f32 {
    if !left_lane_beside_canvas(ui) || ui.tab_hid.is_some() {
        return 0.0;
    }
    let widest = docked(ui, false).iter().filter_map(|&k| ui.custom_toolbars.get(k)).map(bar_width).fold(0.0, f32::max);
    if widest > 0.0 { widest + 2.0 * LANE_GAP } else { 0.0 }
}

/// With Keep Tools Beside the Canvas on and Tools docked on the left, Tools draws inside the canvas
/// panel, right of the panels docked on the left, and the left lane goes right of it there
/// ([`lane_beside_canvas`]).
pub(crate) fn left_lane_beside_canvas(ui: &UiState) -> bool {
    ui.tools_beside_canvas && ui.toolbar && ui.screen_mode < 3 && ui.toolbar_pos.is_none() && !ui.toolbar_right
}

/// The left lane inside the canvas panel, right of the Tools panel docked beside the canvas
/// (`toolbar::show_beside_canvas`, after Tools). Its target reaches from the canvas's left edge,
/// across Tools, over the lane. It wraps into more columns only as far as the canvas keeps its
/// least width, 200 points (`panel_docking`'s canvas limits, which add [`beside_canvas_reserve`]).
pub(crate) fn lane_beside_canvas(app: &VectorcraftApp, ui: &mut egui::Ui) {
    if left_lane_beside_canvas(&app.ui) {
        let avail = ui.available_rect_before_wrap();
        let tools = ui.ctx().data(|d| d.get_temp::<egui::Rect>(crate::toolbar::docked_rect_id()));
        let edge = tools.map_or(avail, |t| t.union(avail));
        lane(app, ui, false, edge, (avail.width() / 2.0).min(avail.width() - 200.0));
    }
}

/// The lane at the right edge (true) or the left of `ui`'s space, at most `room` points wide unless
/// one column is wider, and its target, which reaches out to `edge`'s side.
fn lane(app: &VectorcraftApp, ui: &mut egui::Ui, right: bool, edge: egui::Rect, room: f32) {
    let ctx = ui.ctx().clone();
    let shown = app.ui.tab_hid.is_none() && app.ui.screen_mode < 3;
    let on = if shown { docked(&app.ui, right) } else { vec![] };
    let widest = on.iter().filter_map(|&k| app.ui.custom_toolbars.get(k)).map(bar_width).fold(0.0, f32::max);
    let avail = ui.available_rect_before_wrap();
    let most = most_columns(room, widest);
    let cols = lane_columns(on.len(), avail.height(), most).0 as f32;
    let width = if widest > 0.0 { cols * widest + (cols - 1.0) * LANE_GAP } else { 0.0 };
    let reach = (width + 2.0 * LANE_GAP).max(ZONE);
    let zone = if right {
        egui::Rect::from_min_max(pos2(avail.right() - reach, avail.top()), pos2(edge.right(), avail.bottom()))
    } else {
        egui::Rect::from_min_max(pos2(edge.left(), avail.top()), pos2(avail.left() + reach, avail.bottom()))
    };
    let lane = (width > 0.0).then(|| {
        let panel = if right { egui::Panel::right("custom-toolbar-lane-right") } else { egui::Panel::left("custom-toolbar-lane-left") };
        panel.resizable(false).exact_size(width + 2.0 * LANE_GAP).frame(egui::Frame::NONE).show(ui, |_| {}).response.rect
    });
    if ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(right))) != lane {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| {
        d.insert_temp(zone_id(right), zone);
        match lane {
            Some(lane) => {
                d.insert_temp(lane_id(right), lane);
            }
            None => d.remove::<egui::Rect>(lane_id(right)),
        }
    });
}

/// Where each docked toolbar that shows is drawn, by index: its top-left corner and the most height
/// it takes, and whether it shows only its title bar. Each lane stacks its toolbars from the top in
/// their order, each with an equal share of its column's height; a toolbar shorter than its share
/// leaves the rest to the ones below it. A lane too short for one column wraps its stack into the
/// columns of [`lane_columns`], left to right, as many as the lane is wide enough for, so every
/// toolbar's title bar and tools stay in view; when even those columns give a toolbar less than
/// [`DOCKED_LEAST`] points, the toolbars of that column show only their title bars, which still
/// float or dock them. When a column is too short for even their title bars, it shows as many as
/// fit and the mouse wheel over it scrolls through the rest; those scrolled out of view have no
/// corner. Every toolbar stays inside the lane. A toolbar docked since the lanes were last laid
/// out is not drawn until they are.
fn docked_places(ui: &UiState, ctx: &egui::Context) -> Vec<(usize, Option<egui::Pos2>, f32, bool)> {
    let mut out = vec![];
    // The least height a toolbar takes: its title bar, its border and its bottom margin.
    let least = GRIP + LANE_GAP;
    for right in [false, true] {
        let Some(lane) = ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(right))) else { continue };
        let on = docked(ui, right);
        let widest = on.iter().filter_map(|&k| ui.custom_toolbars.get(k)).map(bar_width).fold(0.0, f32::max);
        let (cols, per) = lane_columns(on.len(), lane.height(), most_columns(lane.width(), widest));
        let pitch = (lane.width() - LANE_GAP) / cols as f32;
        for (c, column) in on.chunks(per).enumerate() {
            let middle = lane.left() + LANE_GAP / 2.0 + pitch * (c as f32 + 0.5);
            // More than even their title bars fit: the column shows as many as fit, and the wheel
            // over the column scrolls through the others, one toolbar a notch.
            let fits = ((lane.height() - LANE_GAP) / (least + LANE_GAP) + 1e-3).floor().max(1.0) as usize;
            let column = if column.len() > fits {
                let id = lane_id(right).with(("first", c));
                let left = lane.left() + LANE_GAP / 2.0 + pitch * c as f32;
                let strip = egui::Rect::from_x_y_ranges(left..=left + pitch, lane.y_range());
                let notch = ctx.input(|i| {
                    let over = i.pointer.hover_pos().is_some_and(|p| strip.contains(p));
                    let wheel = i.events.iter().filter_map(|e| if let egui::Event::MouseWheel { delta, .. } = e { Some(delta.y) } else { None });
                    if over { wheel.sum::<f32>() } else { 0.0 }
                });
                let mut first = ctx.data(|d| d.get_temp::<usize>(id)).unwrap_or(0);
                if notch > 0.0 {
                    first = first.saturating_sub(1);
                } else if notch < 0.0 {
                    first += 1;
                }
                let first = first.min(column.len() - fits);
                ctx.data_mut(|d| d.insert_temp(id, first));
                let (before, rest) = column.split_at(first);
                let (shown, after) = rest.split_at(fits);
                out.extend(before.iter().chain(after).map(|&k| (k, None, 0.0, true)));
                shown
            } else {
                column
            };
            let n = column.len() as f32;
            let share = (lane.height() - LANE_GAP * (n + 1.0)) / n.max(1.0);
            // Too little height for its tools: the title bar alone.
            let bare = share < DOCKED_LEAST;
            let share = share.max(least);
            let mut y = lane.top() + LANE_GAP;
            for &k in column {
                let Some(bar) = ui.custom_toolbars.get(k) else { continue };
                let drawn = ctx.memory(|m| m.area_rect(area_id(&bar.name)));
                let w = drawn.map_or(bar_width(bar) + 2.0, |r| r.width());
                // Drawn last frame taller than its share (its tools scroll only above a least
                // height), it moves up to end at the lane's bottom.
                let tall = drawn.map_or(least, |r| r.height().max(least));
                let top = y.min(lane.bottom() - tall).max(lane.top());
                let room = share.min(lane.bottom() - top).max(least);
                out.push((k, Some(pos2((middle - w / 2.0).round(), top)), room, bare));
                y = top + drawn.map_or(room, |r| r.height().min(room)) + LANE_GAP;
            }
        }
    }
    out
}

/// The edge a title bar released at the pointer docks its toolbar at (true: the right), with the
/// target to light up: none until the pointer has moved [`TRAVEL`] points from the press at `from`.
fn drop_target(ctx: &egui::Context, from: egui::Pos2) -> Option<(egui::Rect, bool)> {
    let at = ctx.input(|i| i.pointer.interact_pos()).filter(|at| at.distance(from) > TRAVEL)?;
    [false, true].into_iter().find_map(|right| ctx.data(|d| d.get_temp::<egui::Rect>(zone_id(right))).filter(|z| z.contains(at)).map(|z| (z, right)))
}

/// Put the toolbar being dragged back where the drag began; returns whether a drag was in progress.
fn cancel(ui: &mut UiState, ctx: &egui::Context) -> bool {
    let Some(mut m) = ctx.data(|d| d.get_temp::<Move>(move_id())).filter(|m| !m.cancelled) else { return false };
    if let Ok(k) = index(ui, &m.name) {
        match m.dock {
            Some(at) => {
                dock_at(&mut ui.custom_toolbars, k, m.right, Some(at));
            }
            None => {
                if let Some(b) = ui.custom_toolbars.get_mut(k) {
                    b.dock = None;
                    b.dock_right = m.right;
                }
                renumber(&mut ui.custom_toolbars);
            }
        }
        if let Some(b) = ui.custom_toolbars.get_mut(k) {
            b.pos = m.pos;
        }
    }
    m.cancelled = true;
    ctx.data_mut(|d| d.insert_temp(move_id(), m));
    true
}

/// Escape while a toolbar's title bar is dragged puts the toolbar back where the drag began, and the
/// press runs no shortcut ([`crate::tools_float::escape_cancelled_move`]).
pub(crate) fn cancel_move_on_escape(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && cancel(&mut app.ui, ctx) {
        crate::tools_float::mark_escape_cancelled(ctx);
    }
}

/// Light up the target a title bar dropped now docks its toolbar at.
fn highlight(ctx: &egui::Context, zone: egui::Rect) {
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("custom-toolbar-drop")));
    painter.rect(zone.shrink(1.0), CornerRadius::same(2), t.accent.gamma_multiply(0.2), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
}

/// The toolbars made with New Toolbar that show, each floating where its title bar was dragged or
/// docked in its edge's lane ([`lanes`]): the title bar's × hides it, its double arrow lays the
/// tools out in one column or two, and its context menu docks or floats it; then its tool buttons
/// (a group's flyout opens as the Tools panel's do), the + button whose menu adds and removes
/// tools, and the ••• button that opens the All Tools drawer beside it.
pub(crate) fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // A drag ends with the press, also for a toolbar hidden while it was dragged.
    let end_move = || {
        if !ctx.input(|i| i.pointer.primary_down()) {
            ctx.data_mut(|d| d.remove::<Move>(move_id()));
        }
    };
    // Tab hides them, also when Window › Tools shows the Tools panel again.
    if app.ui.tab_hid.is_some() || app.ui.custom_toolbars.iter().all(|b| !b.shown) {
        end_move();
        return;
    }
    // Losing the window's focus puts a toolbar being dragged back, as Escape does.
    if !ctx.input(|i| i.focused) {
        cancel(&mut app.ui, ctx);
    }
    let places = docked_places(&app.ui, ctx);
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let active = app.session.tool_id();
    let editing = drawer_target(&app.ui).map(str::to_string);
    let held_id = egui::Id::unique("custom-toolbar-held");
    if ctx.input(|i| i.pointer.any_pressed()) {
        ctx.data_mut(|d| d.remove::<egui::Id>(held_id));
    }
    let held: Option<egui::Id> = ctx.data(|d| d.get_temp(held_id));
    let mut actions = vec![];
    // Where the drawer opens (the toolbar's bounds and its ••• button): beside the toolbar it
    // edits, or beside the one whose ••• button was just clicked.
    let (mut anchor, mut clicked_anchor) = (None, None);
    for k in 0..app.ui.custom_toolbars.len() {
        let Some(bar) = app.ui.custom_toolbars.get(k).filter(|b| b.shown).cloned() else { continue };
        let id = area_id(&bar.name);
        let cols = if bar.double { 2 } else { 1 };
        let width = WIDTH + BUTTON * (cols - 1) as f32;
        // Docked: where its lane puts it, its tools scrolling in its share of the lane, or only its
        // title bar when that share is too small for them.
        let (pos, limit, bare) = if bar.dock.is_some() {
            let Some(&(_, pos, share, bare)) = places.iter().find(|(i, ..)| *i == k) else {
                // Docked since the lanes were laid out: drawn in the next frame.
                ctx.request_repaint();
                continue;
            };
            // Scrolled out of its column's view.
            let Some(pos) = pos else { continue };
            (pos, share - LANE_GAP, bare)
        } else {
            // Kept on screen by its title bar and first tools: the rest scrolls.
            let size = ctx.memory(|m| m.area_rect(id)).map_or(vec2(width, 60.0), |r| vec2(r.width(), r.height().min(160.0)));
            let pos = crate::tools_float::clamp(bar.pos, size, screen);
            (pos, screen.bottom() - pos.y - 12.0, false)
        };
        let edit_this = editing.as_deref() == Some(bar.name.as_str());
        let mut moved = None;
        // The title bar's drag this frame: started, going on, ended.
        let mut drag = (false, false, false);
        let mut drops = None;
        // A Learn link locating a tool of this toolbar: its button, scrolled into view once.
        let locate = crate::panels::learn::locate::wanted(&app.ui, Some(&bar.name));
        let scrolled = app.ui.learn_locate.as_ref().is_some_and(|l| l.scrolled);
        // The toolbar comes to the front once, over other floating toolbars, unless the tool's
        // group is torn off into a strip, which comes to the front instead.
        let holder: Option<Vec<&'static str>> = locate
            .and_then(|tool| bar.slots.iter().find(|s| s.iter().any(|t| t == tool)))
            .map(|slot| slot.iter().filter_map(|id| tool_info(id).map(|t| t.id)).collect());
        let torn_off = holder
            .as_ref()
            .is_some_and(|tools| tools.len() > 1 && tools.first().is_some_and(|key| crate::toolbar::is_floating(app, Some(&bar.name), key)));
        if holder.is_some() && !torn_off && crate::panels::learn::locate::raise_once(&mut app.ui) {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
        }
        let (mut found, mut did_scroll) = (None, false);
        egui::Area::new(id).order(egui::Order::Middle).fixed_pos(pos).show(ctx, |ui| {
            crate::toolbar::flyout_frame(&t, false).inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 4 }).show(ui, |ui| {
                ui.set_width(width);
                // The tools and the ••• button scroll when they don't fit in the window (docked: in
                // its share of the lane) under the title bar.
                ui.set_max_height(limit);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let middle = ui.max_rect().center().x;
                // The title bar: × at its left end, the double arrow at its right end, and the grip
                // between them, whose drag moves the toolbar.
                let (title, _) = ui.allocate_exact_size(vec2(ui.available_width(), GRIP), Sense::hover());
                ui.painter().rect_filled(title, 0.0, t.tab_strip);
                let close = egui::Rect::from_min_size(title.min, vec2(TITLE_BUTTON, GRIP));
                let arrows = egui::Rect::from_min_size(pos2(title.right() - TITLE_BUTTON, title.top()), vec2(TITLE_BUTTON, GRIP));
                let grip = egui::Rect::from_min_max(pos2(close.right(), title.top()), pos2(arrows.left(), title.bottom()));
                let cresp = ui.interact(close, id.with("close"), Sense::click());
                icons::paint(ui, "x", close.shrink2(vec2(1.5, 2.5)), if cresp.hovered() { t.text_strong } else { t.text });
                cresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Close")));
                if cresp.on_hover_text(tl!("Close")).clicked() {
                    actions.push(Action::Hide(bar.name.clone()));
                }
                let aresp = ui.interact(arrows, id.with("columns"), Sense::click());
                let chevrons = if bar.double { "chevrons-left" } else { "chevrons-right" };
                icons::paint(ui, chevrons, arrows.shrink2(vec2(1.0, 2.0)), if aresp.hovered() { t.text_strong } else { t.text });
                aresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Toggle single/double column")));
                if aresp.on_hover_text(tl!("Toggle single/double column")).clicked() {
                    actions.push(Action::Columns(bar.name.clone(), !bar.double));
                }
                // Not focusable (`Sense::drag()` is): only the pointer drags the grip, so Tab does not
                // stop on it. It senses clicks for its context menu.
                let gresp = ui.interact(grip, id.with("grip"), Sense::CLICK | Sense::DRAG);
                for n in 0..3 {
                    let y = grip.center().y - 2.0 + n as f32 * 2.0;
                    let half = (grip.width() / 2.0 - 2.0).clamp(0.0, 8.0);
                    ui.painter().line_segment([pos2(grip.center().x - half, y), pos2(grip.center().x + half, y)], Stroke::new(0.5, t.text_disabled));
                }
                gresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &bar.name));
                // Its context menu: Dock to Left and Dock to Right, the edge it is docked at checked,
                // and Float Toolbar while it is docked.
                gresp.context_menu(|ui| {
                    for (right, label) in [(false, tl!("Dock to Left")), (true, tl!("Dock to Right"))] {
                        let here = bar.dock.is_some() && bar.dock_right == right;
                        if widgets::menu_item(ui, label, !here, here) {
                            actions.push(Action::Dock(bar.name.clone(), right));
                            ui.close();
                        }
                    }
                    if widgets::menu_item(ui, tl!("Float Toolbar"), bar.dock.is_some(), false) {
                        actions.push(Action::Float(bar.name.clone()));
                        ui.close();
                    }
                });
                drag = (gresp.drag_started(), gresp.dragged(), gresp.drag_stopped());
                // Held where the press took it, from the toolbar's corner, while it is dragged.
                let grab = id.with("grab");
                if gresp.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    let (origin, at) = ui.input(|i| (i.pointer.press_origin(), i.pointer.interact_pos()));
                    if let Some(at) = at {
                        let off = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(grab, || origin.unwrap_or(at) - pos));
                        moved = Some(at - off);
                    }
                } else {
                    ui.data_mut(|d| d.remove::<egui::Vec2>(grab));
                    gresp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(&bar.name);
                }
                if bare {
                    return;
                }
                // A gap keeps a press near the grip bar's lower edge on the bar.
                ui.add_space(4.0);
                let mut buttons: Vec<(usize, egui::Rect)> = vec![];
                let edit = widgets::strip_scroll(ui, &format!("custom-toolbar-{}", key(&bar.name)), |ui| {
                    let slots: Vec<(usize, &Vec<String>)> = bar.slots.iter().enumerate().collect();
                    let press = Press { bar: &bar, active, edit_this, held, held_id };
                    for row in slots.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.add_space((middle - BUTTON / 2.0 * row.len() as f32 - ui.cursor().min.x).max(0.0));
                            for &(s, slot) in row {
                                let Some((rect, shown)) = slot_button(ui, &t, &press, slot, &mut actions) else { continue };
                                if edit_this {
                                    buttons.push((s, rect));
                                }
                                if let Some(tool) = locate.filter(|tool| slot.iter().any(|id| id == tool)) {
                                    if !scrolled && !ui.clip_rect().contains_rect(rect) {
                                        ui.scroll_to_rect(rect, None);
                                        did_scroll = true;
                                    }
                                    found = Some((rect, shown == tool, slot.clone(), ui.layer_id(), ui.clip_rect()));
                                }
                            }
                        });
                    }
                    ui.add_space(4.0);
                    // Add Tool…: a menu of the tools, which adds one to this toolbar or takes one out.
                    let add = ui.vertical_centered(|ui| widgets::icon_button(ui, "plus", "Add Tool…", false, 26.0)).inner;
                    // Edit Toolbar: the All Tools drawer opens beside this toolbar, by this button.
                    (add, ui.vertical_centered(|ui| widgets::icon_button(ui, "ellipsis", "Edit Toolbar", edit_this, 26.0)).inner)
                });
                let (add, edit) = edit;
                add_menu(&add, &t, &bar, &mut actions);
                if edit.clicked() {
                    actions.push(Action::EditToolbar(bar.name.clone()));
                }
                // The toolbar as drawn (inside its border): where tools drop, and where a click
                // leaves the drawer open; also on the click that opens the drawer here.
                let bounds = ui.min_rect();
                if edit.clicked() {
                    clicked_anchor = Some((bounds, edit.rect));
                } else if edit_this {
                    anchor = Some((bounds, edit.rect));
                }
                if edit_this {
                    drops = Some((buttons, bounds));
                }
            });
            if let Some((buttons, bounds)) = drops.take() {
                crate::edit_toolbar::toolbar_drop(app, ui, &buttons, cols, bounds, Some(&bar.name));
            }
        });
        drag_title_bar(app, ctx, k, drag, moved);
        // The button is marked when it shows the located tool, else the group's flyout opens on it;
        // a group torn off into a strip is marked there.
        if did_scroll {
            crate::panels::learn::locate::scroll_once(&mut app.ui);
            ctx.request_repaint();
        } else if let Some((rect, shows, slot, layer, clip)) = found {
            let tools: Vec<&'static str> = slot.iter().filter_map(|id| tool_info(id).map(|t| t.id)).collect();
            let torn_off = tools.len() > 1 && tools.first().is_some_and(|key| crate::toolbar::is_floating(app, Some(&bar.name), key));
            if !torn_off {
                if shows {
                    crate::panels::learn::locate::mark(app, ctx, layer, clip, rect);
                } else if crate::panels::learn::locate::reveal_once(&mut app.ui) {
                    actions.push(Action::Flyout(tools, rect, bar.name.clone()));
                }
            }
        }
    }
    if let Some((bounds, button)) = clicked_anchor.or(anchor) {
        ctx.data_mut(|d| {
            d.insert_temp(crate::edit_toolbar::toolbar_id(), bounds);
            d.insert_temp(crate::edit_toolbar::button_id(), button);
        });
    }
    for a in actions {
        match a {
            Action::Select(id) => app.select_tool(id),
            Action::Options(id) => {
                app.select_tool(id);
                app.run("tool.options", json!({ "tool": id })).ok();
            }
            // A group that floats already (one strip per group) is raised instead, as the Tools
            // panel's buttons do.
            Action::Flyout(tools, _, name) if tools.first().is_some_and(|key| crate::toolbar::is_floating(app, Some(&name), key)) => {
                if let Some(key) = tools.first() {
                    crate::toolbar::raise_floating(ctx, Some(&name), key);
                }
            }
            Action::Flyout(tools, rect, name) => {
                app.ui.flyout = Some(0);
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::unique("flyout-anchor"), rect);
                    d.insert_temp(egui::Id::unique("flyout-tools"), tools.iter().map(|s| s.to_string()).collect::<Vec<String>>());
                    d.insert_temp(crate::toolbar::flyout_toolbar_id(), Some(name));
                });
            }
            Action::EditToolbar(name) => {
                if let Err(e) = app.run("window.editToolbar", json!({ "toolbar": name })) {
                    app.status(e);
                }
            }
            Action::AddTool(name, tool) => {
                if let Err(e) = app.run("toolbar.addTool", json!({ "tool": tool, "toolbar": name })) {
                    app.status(e);
                }
            }
            Action::RemoveTool(name, tool) => {
                if let Err(e) = app.run("toolbar.removeTool", json!({ "tool": tool, "toolbar": name })) {
                    app.status(e);
                }
            }
            Action::Hide(name) => {
                if let Err(e) = app.run("window.toolbar.custom", json!({ "name": name, "show": false })) {
                    app.status(e);
                }
            }
            Action::Columns(name, double) => {
                if let Err(e) = app.run("window.toolbarColumns", json!({ "toolbar": name, "double": double })) {
                    app.status(e);
                }
            }
            Action::Dock(name, right) => {
                if let Err(e) = app.run("window.toolbar.dock", json!({ "name": name, "side": if right { "right" } else { "left" } })) {
                    app.status(e);
                }
            }
            Action::Float(name) => {
                if let Err(e) = app.run("window.toolbar.float", json!({ "name": name })) {
                    app.status(e);
                }
            }
        }
    }
    end_move();
}

/// The drag of the title bar of the toolbar at `k` this frame (`drag`: started, going on, ended),
/// with the pointer holding its corner at `moved`. A floating toolbar follows the pointer, and the
/// target under the pointer lights up; released there, it docks at the end of that edge's stack. A
/// docked toolbar stays in its lane until the pointer leaves the lane, and then floats under the
/// pointer, also when the press is released in the frame it leaves the lane.
/// After Escape, the rest of the drag moves nothing.
fn drag_title_bar(app: &mut VectorcraftApp, ctx: &egui::Context, k: usize, drag: (bool, bool, bool), moved: Option<egui::Pos2>) {
    let (started, dragging, stopped) = drag;
    let Some(bar) = app.ui.custom_toolbars.get(k) else { return };
    if started && let Some(from) = ctx.input(|i| i.pointer.press_origin().or(i.pointer.interact_pos())) {
        let grab = ctx.memory(|m| m.area_rect(area_id(&bar.name))).map_or(egui::Vec2::ZERO, |r| from - r.min);
        let m = Move { name: bar.name.clone(), from, grab, dock: bar.dock, right: bar.dock_right, pos: bar.pos, cancelled: false };
        ctx.data_mut(|d| d.insert_temp(move_id(), m));
    }
    let Some(m) = ctx.data(|d| d.get_temp::<Move>(move_id())).filter(|m| !m.cancelled && key(&m.name) == key(&bar.name)) else {
        return;
    };
    let from = m.from;
    let mut docked = bar.dock.is_some();
    if docked && (dragging || stopped) {
        let at = ctx.input(|i| i.pointer.interact_pos());
        // Where the pointer holds its corner; on the release frame, which reports no move, from
        // where the press took it.
        let corner = moved.or_else(|| at.map(|at| at - m.grab)).filter(|p| p.x.is_finite() && p.y.is_finite());
        let lane = ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(bar.dock_right))).or_else(|| ctx.memory(|m| m.area_rect(area_id(&bar.name))));
        let inside = lane.zip(at).is_some_and(|(r, at)| r.contains(at));
        if !inside && let Some(p) = corner {
            if let Some(b) = app.ui.custom_toolbars.get_mut(k) {
                b.dock = None;
                b.pos = [p.x, p.y];
            }
            renumber(&mut app.ui.custom_toolbars);
            docked = false;
        }
    } else if dragging
        && let Some(p) = moved.filter(|p| p.x.is_finite() && p.y.is_finite())
        && let Some(b) = app.ui.custom_toolbars.get_mut(k)
    {
        b.pos = [p.x, p.y];
    }
    if docked {
        return;
    }
    let target = drop_target(ctx, from);
    if dragging && let Some((zone, _)) = target {
        highlight(ctx, zone);
    }
    if stopped && let Some((_, right)) = target {
        dock_at(&mut app.ui.custom_toolbars, k, right, None);
    }
}

/// What a toolbar's tool buttons need from the frame: the toolbar, the active tool, whether the
/// drawer edits the toolbar, and the button whose long press opened its flyout.
struct Press<'a> {
    bar: &'a CustomToolbar,
    active: &'static str,
    edit_this: bool,
    held: Option<egui::Id>,
    held_id: egui::Id,
}

/// One slot's tool button, as the Tools panel's: it shows the tool chosen last from its group, a
/// press selects it, and a long or right press opens the group's flyout; while the drawer edits the
/// toolbar, it drags instead. Returns where it is drawn and the tool it shows.
fn slot_button(ui: &mut egui::Ui, t: &Tokens, p: &Press, slot: &[String], actions: &mut Vec<Action>) -> Option<(egui::Rect, &'static str)> {
    let tools: Vec<&'static str> = slot.iter().filter_map(|id| tool_info(id).map(|t| t.id)).collect();
    let is_active = tools.contains(&p.active);
    let shown = tool_info(shown_tool(p.bar, p.active, &tools)?)?;
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON, crate::toolbar::PITCH - 1.0), Sense::click_and_drag());
    let well = egui::Rect::from_center_size(rect.center(), vec2(30.0, 27.5));
    if is_active {
        ui.painter().rect_filled(well, CornerRadius::same(6), t.tool_active);
    } else if resp.hovered() {
        ui.painter().rect_filled(well, CornerRadius::same(6), t.hover);
    }
    let ir = egui::Rect::from_center_size(rect.center(), vec2(18.0, 18.0));
    icons::paint(ui, icons::tool_icon(shown.icon), ir, if is_active { t.text_strong } else { t.icon });
    if tools.len() > 1 {
        let c = rect.center() + vec2(12.5, 10.0);
        ui.painter().add(egui::Shape::convex_polygon(vec![c, c + vec2(-3.5, 0.0), c + vec2(0.0, -3.5)], t.icon, Stroke::NONE));
    }
    if p.edit_this {
        resp.dnd_set_drag_payload(crate::edit_toolbar::ToolDrag { tool: shown.id, from_toolbar: true });
    }
    let press = if tools.len() > 1 && !p.edit_this { crate::toolbar::flyout_press(ui, &resp, rect) } else { None };
    let alt = ui.input(|i| i.modifiers.alt);
    if press.is_some() {
        actions.push(Action::Flyout(tools.clone(), rect, p.bar.name.clone()));
        if press == Some(crate::toolbar::FlyoutPress::Primary) {
            ui.data_mut(|d| d.insert_temp(p.held_id, resp.id));
        }
    } else if resp.clicked() && p.held == Some(resp.id) {
        // Releasing the long press that opened the flyout leaves it open.
    } else if resp.clicked() && alt && tools.len() > 1 {
        let i = tools.iter().position(|x| *x == shown.id).unwrap_or(0);
        actions.push(Action::Select(tools[(i + 1) % tools.len()]));
    } else if resp.double_clicked() {
        actions.push(Action::Options(shown.id));
    } else if resp.clicked() {
        actions.push(Action::Select(shown.id));
    }
    let tip = crate::toolbar::tip(shown);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), is_active, &tip));
    resp.on_hover_text(tip);
    Some((rect, shown.id))
}

/// The tool a slot's button on `bar` shows, of the slot's known `tools`: the active tool when the
/// slot holds it, else the one chosen last from the slot's group, else its first; none for a slot
/// with no known tool.
pub(crate) fn shown_tool(bar: &CustomToolbar, active: &'static str, tools: &[&'static str]) -> Option<&'static str> {
    let &first = tools.first()?;
    if tools.contains(&active) {
        return Some(active);
    }
    Some(bar.slot_tool.get(first).and_then(|id| tools.iter().copied().find(|t| t == id)).unwrap_or(first))
}

/// The area of Add Tool…'s menu on the toolbar named `name`.
pub(crate) fn add_menu_id(name: &str) -> egui::Id {
    egui::Id::unique(("custom-toolbar-add", key(name)))
}

/// Add Tool…'s menu, which the + button on the toolbar `bar` opens: under Remove Tool the tools in
/// the toolbar, where a click takes one out; then every tool under the All Tools drawer's headings,
/// where a click adds one at the end (dimmed: in the toolbar already). It scrolls past the window's
/// bottom bars, as other long menus do ([`widgets::menu_scroll`]).
fn add_menu(button: &egui::Response, t: &Tokens, bar: &CustomToolbar, actions: &mut Vec<Action>) {
    egui::Popup::menu(button).id(add_menu_id(&bar.name)).show(|ui| {
        ui.set_min_width(200.0);
        widgets::menu_scroll(ui, |ui| {
            let held: Vec<&'static str> = bar.slots.iter().flatten().filter_map(|id| tool_info(id).map(|t| t.id)).collect();
            let heading = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(text).size(12.0).color(t.text_strong));
            if !held.is_empty() {
                heading(ui, &format!("{}:", tl!("Remove Tool")));
                for tool in held.iter().filter_map(|id| tool_info(id)) {
                    if widgets::menu_item(ui, tool.label, true, false) {
                        actions.push(Action::RemoveTool(bar.name.clone(), tool.id));
                    }
                }
                ui.separator();
            }
            for (cat, tools) in crate::edit_toolbar::sections() {
                heading(ui, &format!("{}:", tl!(cat)));
                for tool in tools {
                    if widgets::menu_item(ui, tool.label, !held.contains(&tool.id), false) {
                        actions.push(Action::AddTool(bar.name.clone(), tool.id));
                    }
                }
            }
        });
    });
}

/// Remember `id` as the tool shown by each slot of a toolbar made with New Toolbar that holds it,
/// however it was chosen (`toolbar::remember` calls it).
pub(crate) fn remember(ui: &mut UiState, id: &str) {
    for bar in &mut ui.custom_toolbars {
        let firsts: Vec<String> = bar.slots.iter().filter(|s| s.len() > 1 && s.iter().any(|t| t == id)).filter_map(|s| s.first().cloned()).collect();
        for first in firsts {
            bar.slot_tool.insert(first, id.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;

    use super::*;
    use crate::menus::Item;

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(Session::new(), Default::default())
    }

    /// The items of Window › Toolbars: (label, command id, params).
    fn toolbars_menu(app: &VectorcraftApp) -> Vec<(&'static str, &'static str, Value)> {
        crate::shortcut_editor::sync(&app.ui);
        let window = crate::menus::menu_tree().into_iter().find(|(m, _)| *m == "Window").map(|(_, items)| items).unwrap();
        let Some(Item::Sub(_, items)) = window.into_iter().find(|i| matches!(i, Item::Sub("Toolbars", _))) else { panic!("no Toolbars submenu") };
        items.into_iter().filter_map(|i| if let Item::Cmd(l, id, p) = i { Some((l, id, p)) } else { None }).collect()
    }

    /// New Toolbar named "Mine" is listed under Window › Toolbars, checked while it shows; its item
    /// hides and shows it. Without a name, a dialog asks for one.
    #[test]
    fn a_new_toolbar_is_listed_under_window_toolbars() {
        let mut app = app();
        let last: Vec<_> = toolbars_menu(&app).iter().rev().take(2).map(|(l, id, _)| (*l, *id)).collect();
        assert_eq!(last, [("Manage Toolbars…", "window.toolbar.manage"), ("New Toolbar…", "window.toolbar.new")]);
        assert_eq!(app.run("window.toolbar.new", json!({"name": " Mine "})).unwrap(), json!({"toolbar": "Mine"}));
        let menu = toolbars_menu(&app);
        let item = menu.iter().find(|(l, ..)| *l == "Mine").expect("Mine is listed");
        assert_eq!((item.1, &item.2), ("window.toolbar.custom", &json!({"name": "Mine"})));
        assert_eq!(crate::menus::checked(&app, item.1, &item.2), Some(true));
        assert_eq!(crate::menus::display_label(&app, item.1, item.0), "Mine", "listed by its own name");
        // A blank toolbar, floating, shown.
        let bar = find(&app.ui, "mine").unwrap();
        assert!(bar.slots.is_empty() && bar.shown);
        // Its item hides it and shows it again.
        assert_eq!(app.run("window.toolbar.custom", json!({"name": "Mine"})).unwrap(), json!(false));
        assert_eq!(crate::menus::checked(&app, item.1, &item.2), Some(false));
        assert_eq!(app.run("window.toolbar.custom", json!({"name": "MINE", "show": true})).unwrap(), json!(true));
        // Names: not empty, not taken whatever the case.
        assert!(app.run("window.toolbar.new", json!({"name": "  "})).unwrap_err().contains("empty"));
        assert!(app.run("window.toolbar.new", json!({"name": "mine"})).unwrap_err().contains("already"));
        // Whatever the case outside ASCII too: one name, one toolbar.
        app.run("window.toolbar.new", json!({"name": "É"})).unwrap();
        assert!(app.run("window.toolbar.new", json!({"name": "é"})).unwrap_err().contains("already"));
        assert!(app.run("window.toolbar.custom", json!({"name": "é", "show": true})).is_ok());
        app.ui.custom_toolbars.retain(|b| b.name != "É");
        assert!(app.run("window.toolbar.custom", json!({"name": "Nope"})).unwrap_err().contains("Nope"));
        // A name is looked up without its leading and trailing spaces, as it was made.
        assert_eq!(app.run("window.toolbar.custom", json!({"name": " Mine ", "show": true})).unwrap(), json!(true));
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine "})).is_ok());
        assert!(app.run("toolbar.removeTool", json!({"tool": "pen", "toolbar": "Mine"})).is_ok());
        // A name that is not a string is reported as one.
        assert!(app.run("window.toolbar.new", json!({"name": 5})).unwrap_err().contains("must be a string"));
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": 5})).unwrap_err().contains("must be a string"));
        // Without a name: the dialog, its OK makes the toolbar.
        assert_eq!(app.run("window.toolbar.new", json!({})).unwrap(), Value::Null);
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.kind.as_str(), d.str("name").as_str()), (KIND, "Toolbar 1"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let names: Vec<&str> = toolbars_menu(&app).into_iter().filter(|(_, id, _)| *id == "window.toolbar.custom").map(|(l, ..)| l).collect();
        assert_eq!(names, ["Mine", "Toolbar 1"]);
        let list = app.run("window.toolbar.list", json!({})).unwrap();
        let blank = |name: &str| json!({"name": name, "shown": true, "double": false, "slots": [], "floating": true, "side": "left", "order": null});
        assert_eq!(list, json!([blank("Mine"), blank("Toolbar 1")]));
    }

    /// The New Toolbar dialog: a name typed in it and Enter make the toolbar; a name that is taken is
    /// reported in the status line and the dialog stays open; OK makes it once the name is free, and
    /// Cancel closes the dialog without a toolbar.
    #[test]
    fn the_new_toolbar_dialog_takes_the_name_typed() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0));
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| show_dialog(app, ui.ctx()));
            out.textures_delta.clear();
            out.shapes
                .iter()
                .filter_map(
                    |c| if let egui::Shape::Text(t) = &c.shape { Some((t.galley.text().to_string(), t.visual_bounding_rect())) } else { None },
                )
                .collect::<Vec<_>>()
        };
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let button =
            |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let at =
            |texts: &[(String, egui::Rect)], s: &str| texts.iter().find(|(t, _)| t == s).unwrap_or_else(|| panic!("no {s} in the dialog")).1.center();
        // Typed into the Name field, then Enter. The dialog is sized in its first frame and drawn
        // from the second.
        app.run("window.toolbar.new", json!({})).unwrap();
        frame(&mut app, vec![]);
        let texts = frame(&mut app, vec![]);
        let field = at(&texts, "Toolbar 1");
        frame(&mut app, vec![egui::Event::PointerMoved(field), button(field, true)]);
        frame(&mut app, vec![button(field, false)]);
        frame(&mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text("Mine".into())]);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("name")).as_deref(), Some("Mine"));
        frame(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(app.ui.dialog.is_none(), "Enter makes the toolbar");
        assert!(find(&app.ui, "Mine").is_some());
        // A name that is taken: the status line says so, and the dialog stays open.
        app.run("window.toolbar.new", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("MINE"));
        frame(&mut app, vec![]);
        let texts = frame(&mut app, vec![]);
        let ok = at(&texts, "OK");
        frame(&mut app, vec![egui::Event::PointerMoved(ok), button(ok, true)]);
        frame(&mut app, vec![button(ok, false)]);
        assert!(app.ui.status.contains("already"), "{}", app.ui.status);
        assert!(app.ui.dialog.is_some(), "the dialog stays open");
        // OK with a free name makes it.
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Other"));
        frame(&mut app, vec![egui::Event::PointerMoved(ok), button(ok, true)]);
        frame(&mut app, vec![button(ok, false)]);
        assert!(app.ui.dialog.is_none() && find(&app.ui, "Other").is_some());
        // Cancel makes none.
        app.run("window.toolbar.new", json!({})).unwrap();
        frame(&mut app, vec![]);
        let texts = frame(&mut app, vec![]);
        let cancel = at(&texts, "Cancel");
        frame(&mut app, vec![egui::Event::PointerMoved(cancel), button(cancel, true)]);
        frame(&mut app, vec![button(cancel, false)]);
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.ui.custom_toolbars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Mine", "Other"]);
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
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        let mut ui = prefs.sanitized();
        assert_eq!(find(&ui, "Mine"), Some(&mine));
        crate::workspaces::switch(&mut ui, "Essentials").unwrap();
        crate::workspaces::switch(&mut ui, "Drawing").unwrap();
        assert_eq!(find(&ui, "Mine"), Some(&mine));
        // Reset makes it blank again.
        assert_eq!(app.run("toolbar.reset", json!({"toolbar": "Mine"})).unwrap(), json!([]));
        // Workspaces saved before New Toolbar load without toolbars.
        let mut old = serde_json::to_value(crate::workspaces::capture(&app.ui, "Old")).unwrap();
        old.as_object_mut().unwrap().remove("toolbars");
        assert!(serde_json::from_value::<crate::workspaces::Workspace>(old).unwrap().toolbars.is_empty());
    }

    /// One frame of the whole window, `height` points tall.
    fn window_frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, height: f32) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, height));
        let raw = egui::RawInput { time: Some(time), screen_rect: Some(screen), ..Default::default() };
        ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        })
        .textures_delta
        .clear();
    }

    fn drawn(ctx: &egui::Context, id: egui::Id, order: egui::Order) -> bool {
        ctx.memory(|m| m.areas().visible_last_frame(&egui::LayerId::new(order, id)))
    }

    /// Window › Tools hides the Tools panel alone: a toolbar made with New Toolbar still shows, and
    /// the drawer opens beside it. Tab hides it with the other panels, and a second Tab shows it.
    #[test]
    fn a_new_toolbar_shows_without_the_tools_panel_until_tab() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        let mut time = 0.0;
        let mut frames = |app: &mut VectorcraftApp| {
            for _ in 0..3 {
                time += 0.05;
                window_frame(app, &ctx, time, 800.0);
            }
        };
        frames(&mut app);
        assert!(drawn(&ctx, area_id("Mine"), egui::Order::Middle));
        app.run("window.toolbar", json!({})).unwrap();
        assert!(!app.ui.toolbar);
        app.run("window.editToolbar", json!({"toolbar": "Mine"})).unwrap();
        frames(&mut app);
        assert!(drawn(&ctx, area_id("Mine"), egui::Order::Middle), "shown without the Tools panel");
        assert!(drawn(&ctx, crate::edit_toolbar::drawer_id(), egui::Order::Foreground), "the drawer beside it");
        app.run("window.editToolbar", json!({"open": false})).unwrap();
        // A group torn off this toolbar shows too; one torn off the Tools panel stays hidden with it.
        let strip = |tools: &[&str]| crate::state::FloatingFlyout { tools: tools.iter().map(|s| s.to_string()).collect(), pos: [400.0, 300.0] };
        find_mut(&mut app.ui, "Mine").unwrap().floating = vec![strip(&["pen", "curvature"])];
        app.ui.floating_flyouts = vec![strip(&["rectangle", "star"])];
        frames(&mut app);
        assert!(drawn(&ctx, egui::Id::unique(("tool-floating", "pen")).with(key("Mine")), egui::Order::Middle));
        assert!(!drawn(&ctx, egui::Id::unique(("tool-floating", "rectangle")), egui::Order::Middle));
        // Hidden with the toolbar it was torn off, and shown again with it; an open flyout closes.
        app.ui.flyout = Some(0);
        app.run("window.toolbar.custom", json!({"name": "Mine", "show": false})).unwrap();
        assert_eq!(app.ui.flyout, None, "the flyout closes with the toolbar");
        frames(&mut app);
        assert!(!drawn(&ctx, egui::Id::unique(("tool-floating", "pen")).with(key("Mine")), egui::Order::Middle));
        app.run("window.toolbar.custom", json!({"name": "Mine", "show": true})).unwrap();
        frames(&mut app);
        assert!(drawn(&ctx, egui::Id::unique(("tool-floating", "pen")).with(key("Mine")), egui::Order::Middle));
        app.run("window.dock", json!({})).unwrap();
        frames(&mut app);
        assert!(!drawn(&ctx, area_id("Mine"), egui::Order::Middle), "Tab hides it");
        app.run("window.dock", json!({})).unwrap();
        frames(&mut app);
        assert!(drawn(&ctx, area_id("Mine"), egui::Order::Middle), "a second Tab shows it");
        // With the dock, the Tools panel and the Control bar all off, Tab still hides it first, and
        // a second Tab shows it alone.
        (app.ui.dock, app.ui.toolbar, app.ui.control_bar) = (false, false, false);
        app.run("window.dock", json!({})).unwrap();
        frames(&mut app);
        assert!(!drawn(&ctx, area_id("Mine"), egui::Order::Middle), "Tab hides it with the other bars off");
        assert!(!app.ui.dock && !app.ui.toolbar && !app.ui.control_bar);
        app.run("window.dock", json!({})).unwrap();
        frames(&mut app);
        assert!(drawn(&ctx, area_id("Mine"), egui::Order::Middle), "a second Tab shows it again");
        assert!(!app.ui.dock && !app.ui.toolbar && !app.ui.control_bar, "and only it");
        // Hidden with its item while Tab hides it, it stays hidden, and Tab turns no other bar on.
        app.run("window.dock", json!({})).unwrap();
        app.run("window.toolbar.custom", json!({"name": "Mine", "show": false})).unwrap();
        app.run("window.dock", json!({})).unwrap();
        frames(&mut app);
        assert!(!drawn(&ctx, area_id("Mine"), egui::Order::Middle));
        assert!(!app.ui.dock && !app.ui.toolbar && !app.ui.control_bar, "the layout Tab hid comes back");
        // Shown with its item after Tab hid the toolbars, it shows alone, as Window › Tools shows
        // the Tools panel; the other toolbar Tab hid is unchecked, and the next Tab hides it.
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        app.run("window.dock", json!({})).unwrap();
        assert!(app.ui.tab_hid.is_some());
        app.run("window.toolbar.custom", json!({"name": "Mine", "show": true})).unwrap();
        frames(&mut app);
        assert!(drawn(&ctx, area_id("Mine"), egui::Order::Middle), "shown with its item after Tab");
        assert!(!drawn(&ctx, area_id("Other"), egui::Order::Middle));
        assert!(!find(&app.ui, "Other").unwrap().shown, "the menu shows what is on screen");
        app.run("window.dock", json!({})).unwrap();
        frames(&mut app);
        assert!(!drawn(&ctx, area_id("Mine"), egui::Order::Middle), "the next Tab hides it");
        // A flyout of the Tools panel closes when Window › Tools hides the panel; one of a toolbar
        // made with New Toolbar stays open.
        app.run("window.dock", json!({})).unwrap();
        app.ui.toolbar = true;
        frames(&mut app);
        let open = |app: &mut VectorcraftApp, toolbar: Option<String>| {
            app.ui.flyout = Some(0);
            // A group not torn off as a strip of the same toolbar.
            let group = if toolbar.is_some() { ["rectangle", "ellipse"] } else { ["pen", "curvature"] };
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::unique("flyout-tools"), group.map(String::from).to_vec());
                d.insert_temp(crate::toolbar::flyout_toolbar_id(), toolbar);
            });
        };
        open(&mut app, None);
        app.run("window.toolbar", json!({})).unwrap();
        assert!(!app.ui.toolbar);
        frames(&mut app);
        assert_eq!(app.ui.flyout, None, "the hidden Tools panel's flyout closes");
        assert!(!drawn(&ctx, egui::Id::unique("tool-flyout"), egui::Order::Foreground));
        open(&mut app, Some("Mine".into()));
        frames(&mut app);
        assert_eq!(app.ui.flyout, Some(0), "a toolbar's own flyout stays open");
        app.ui.flyout = None;
        // Window › Tools after Tab shows the Tools panel alone; a toolbar made with New Toolbar
        // then shows alone too.
        app.run("window.dock", json!({})).unwrap();
        app.run("window.toolbar", json!({})).unwrap();
        frames(&mut app);
        assert!(app.ui.toolbar && !drawn(&ctx, area_id("Mine"), egui::Order::Middle), "Window › Tools shows only the Tools panel");
        app.run("window.toolbar.new", json!({"name": "Third"})).unwrap();
        frames(&mut app);
        assert!(drawn(&ctx, area_id("Third"), egui::Order::Middle), "a new toolbar shows after Tab");
        assert!(!drawn(&ctx, area_id("Mine"), egui::Order::Middle) && !find(&app.ui, "Mine").unwrap().shown);
    }

    /// A group's button shows the tool chosen last from it however it was chosen (its flyout, a
    /// shortcut), and the Tools panel's buttons keep their own.
    #[test]
    fn a_group_shows_the_tool_chosen_last() {
        let mut app = app();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "curvature", "toolbar": "Mine", "slot": 0, "place": "group"})).unwrap();
        app.select_tool("curvature");
        app.select_tool("selection");
        assert_eq!(find(&app.ui, "Mine").unwrap().slot_tool.get("pen").map(String::as_str), Some("curvature"));
        assert_ne!(app.ui.slot_tool.get("pen").map(String::as_str), Some("curvature"), "the Tools panel's Pen group holds no Curvature tool");
        // Kept with the toolbar.
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        assert_eq!(find(&prefs.sanitized(), "Mine").unwrap().slot_tool.get("pen").map(String::as_str), Some("curvature"));
    }

    /// A group of a toolbar made with New Toolbar floats on its own, apart from a Tools panel group
    /// with the same first tool: both strips show, and each toolbar finds only its own.
    #[test]
    fn groups_float_apart_from_the_tools_panels() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "curvature", "toolbar": "Mine", "slot": 0, "place": "group"})).unwrap();
        app.run("window.floatTools", json!({"tool": "pen", "floating": true})).unwrap();
        assert!(crate::toolbar::is_floating(&app, None, "pen"));
        assert!(!crate::toolbar::is_floating(&app, Some("Mine"), "pen"), "the Tools panel's Pen group floats, not Mine's");
        assert_eq!(app.run("window.floatTools", json!({"tool": "curvature", "toolbar": "mine"})).unwrap(), json!(true));
        assert!(crate::toolbar::is_floating(&app, Some("Mine"), "pen"));
        assert_eq!(crate::menus::checked(&app, "window.floatTools", &json!({"tool": "curvature", "toolbar": "Mine"})), Some(true));
        assert!(app.run("window.floatTools", json!({"tool": "selection", "toolbar": "Mine"})).is_err(), "not in a group of Mine");
        assert!(app.run("window.floatTools", json!({"tool": "pen", "toolbar": "Nope"})).is_err());
        // Both kept by the preferences, Mine's with Mine: a build without New Toolbar reads the
        // Tools panel's strips alone. Both are drawn.
        let saved = serde_json::to_value(&app.ui).unwrap();
        let tools_panel = &app.ui.floating_flyouts[0].tools;
        assert_eq!(saved["floating_flyouts"], json!([{"tools": tools_panel, "pos": [90.0, 100.0]}]));
        let prefs = serde_json::from_value::<UiState>(saved).unwrap().sanitized();
        assert_eq!(prefs.floating_flyouts.len(), 1);
        assert_eq!(find(&prefs, "Mine").unwrap().floating.iter().map(|f| f.tools.clone()).collect::<Vec<_>>(), [["pen", "curvature"]]);
        for k in 0..3 {
            window_frame(&mut app, &ctx, f64::from(k) * 0.05, 800.0);
        }
        let main = egui::Id::unique(("tool-floating", "pen"));
        assert!(drawn(&ctx, main, egui::Order::Middle) && drawn(&ctx, main.with(key("Mine")), egui::Order::Middle));
        // Putting the Tools panel's group back leaves Mine's floating.
        app.run("window.floatTools", json!({"tool": "pen", "floating": false})).unwrap();
        assert!(app.ui.floating_flyouts.is_empty() && find(&app.ui, "Mine").unwrap().floating.len() == 1);
        assert_eq!(crate::menus::checked(&app, "window.floatTools", &json!({"tool": "pen"})), Some(false));
        // And Mine's goes back by its command too.
        assert_eq!(app.run("window.floatTools", json!({"tool": "pen", "toolbar": "Mine"})).unwrap(), json!(false));
        assert!(find(&app.ui, "Mine").unwrap().floating.is_empty());
    }

    /// A tool taken off a toolbar made with New Toolbar leaves the group torn off it too, and the
    /// strip of a group the toolbar no longer holds goes, as Reset blanks the toolbar.
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
        let strips = |app: &VectorcraftApp| find(&app.ui, "Mine").unwrap().floating.iter().map(|f| f.tools.join(",")).collect::<Vec<_>>();
        assert_eq!(strips(&app), ["pen,curvature,rectangle"]);
        app.run("toolbar.removeTool", json!({"tool": "curvature", "toolbar": "Mine"})).unwrap();
        assert_eq!(strips(&app), ["pen,rectangle"]);
        app.run("toolbar.reset", json!({"toolbar": "Mine"})).unwrap();
        assert!(strips(&app).is_empty() && !crate::toolbar::is_floating(&app, Some("Mine"), "pen"));
    }

    /// A toolbar's name has at most [`MAX_NAME`] characters.
    #[test]
    fn a_toolbar_name_past_the_limit_is_refused() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let long = "x".repeat(MAX_NAME + 1);
        assert!(app.run("window.toolbar.new", json!({ "name": long })).is_err());
        assert!(app.ui.custom_toolbars.is_empty());
        app.run("window.toolbar.new", json!({ "name": &long[1..] })).unwrap();
        assert_eq!(app.ui.custom_toolbars.len(), 1);
    }

    /// A toolbar with more tools than the window's height holds stays in the window: its tools
    /// scroll.
    #[test]
    fn a_tall_toolbar_scrolls_inside_the_window() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Tall"})).unwrap();
        for tool in vectorcraft_tools::TOOL_GROUPS.iter().flat_map(|g| g.iter()).take(30) {
            app.run("toolbar.addTool", json!({"tool": tool.id, "toolbar": "Tall"})).unwrap();
        }
        assert_eq!(find(&app.ui, "Tall").unwrap().slots.len(), 30);
        for k in 0..4 {
            window_frame(&mut app, &ctx, f64::from(k) * 0.05, 500.0);
        }
        let r = ctx.memory(|m| m.area_rect(area_id("Tall"))).unwrap();
        assert!(r.bottom() <= 500.0 && r.top() >= 0.0, "{r:?}");
    }

    /// Preferences › Devices › Enable Touch Workspace switches to Touch and back as Window ›
    /// Workspace does: the toolbars made in the workspace left come back with it.
    #[test]
    fn the_touch_workspace_brings_the_toolbars_back() {
        let mut app = app();
        app.run("window.workspace", json!({"name": "Typography"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.session.prefs.touch_workspace = true;
        crate::workspaces::sync_touch_workspace(&mut app, true);
        assert_eq!(app.ui.workspace, crate::workspaces::TOUCH);
        assert!(app.ui.custom_toolbars.is_empty());
        app.session.prefs.touch_workspace = false;
        crate::workspaces::sync_touch_workspace(&mut app, true);
        assert_eq!(app.ui.workspace, "Typography");
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
        assert!(app.ui.toolbar, "the layout is reset");
        assert_eq!(find(&app.ui, "Mine").map(|b| b.slots.clone()), Some(vec![vec!["pen".to_string()]]), "Mine stays in Essentials");
        let mine = find(&app.ui, "Mine").cloned().unwrap();
        // The layout changes go into the new workspace; Essentials keeps only its toolbars.
        app.run("window.toolbar", json!({})).unwrap();
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        assert_eq!(find(&app.ui, "Mine"), Some(&mine), "Drawing starts with a copy");
        app.run("window.toolbar.new", json!({"name": "Only in Drawing"})).unwrap();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(app.ui.toolbar, "Essentials as saved");
        assert_eq!(app.ui.custom_toolbars, std::slice::from_ref(&mine), "with its own toolbars");
        app.run("window.workspace", json!({"name": "Drawing"})).unwrap();
        assert_eq!(app.ui.custom_toolbars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Mine", "Only in Drawing"]);
    }

    /// The title bar's × hides the toolbar, and its double arrow lays its tools out in two columns
    /// and back in one (`window.toolbarColumns` with `toolbar`).
    #[test]
    fn the_title_bar_hides_the_toolbar_and_toggles_two_columns() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        for tool in ["pen", "rectangle", "selection"] {
            app.run("toolbar.addTool", json!({"tool": tool, "toolbar": "Mine"})).unwrap();
        }
        let mut time = 0.0;
        let mut frames = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            for e in std::iter::once(events).chain(std::iter::repeat_with(Vec::new)).take(3) {
                time += 0.05;
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0));
                let raw = egui::RawInput { time: Some(time), screen_rect: Some(screen), events: e, ..Default::default() };
                ctx.run_ui(raw, |ui| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                })
                .textures_delta
                .clear();
            }
        };
        let click = |p: egui::Pos2| {
            let button =
                |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            vec![egui::Event::PointerMoved(p), button(true), button(false)]
        };
        frames(&mut app, vec![]);
        let r = ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap();
        let one = r.width();
        // The double arrow at the right end of the title bar.
        frames(&mut app, click(pos2(r.right() - 1.0 - TITLE_BUTTON / 2.0, r.top() + 1.0 + GRIP / 2.0)));
        assert!(find(&app.ui, "Mine").unwrap().double);
        assert_eq!(crate::menus::checked(&app, "window.toolbarColumns", &json!({"toolbar": "Mine"})), Some(true));
        assert!(!app.ui.toolbar_double, "the Tools panel keeps one column");
        frames(&mut app, vec![]);
        let r = ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap();
        assert!((r.width() - one - BUTTON).abs() < 0.5, "{one} then {}", r.width());
        assert_eq!(app.run("window.toolbarColumns", json!({"toolbar": "mine"})).unwrap(), json!(false));
        assert!(app.run("window.toolbarColumns", json!({"toolbar": "Nope"})).is_err());
        // The × at the left end hides it.
        frames(&mut app, vec![]);
        let r = ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap();
        frames(&mut app, click(pos2(r.left() + 1.0 + TITLE_BUTTON / 2.0, r.top() + 1.0 + GRIP / 2.0)));
        assert!(!find(&app.ui, "Mine").unwrap().shown);
        assert!(!drawn(&ctx, area_id("Mine"), egui::Order::Middle));
    }

    fn listed(app: &VectorcraftApp) -> Vec<&'static str> {
        toolbars_menu(app).into_iter().filter(|(_, id, _)| *id == "window.toolbar.custom").map(|(l, ..)| l).collect()
    }

    /// Manage Toolbars renames, copies and deletes the toolbars made with New Toolbar: each change
    /// shows under Window › Toolbars and stays in the workspace after switching away and back.
    #[test]
    fn manage_toolbars_renames_duplicates_and_deletes() {
        let mut app = app();
        let menu = toolbars_menu(&app);
        assert_eq!(menu.last().map(|(l, id, _)| (*l, *id)), Some(("Manage Toolbars…", "window.toolbar.manage")));
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
        assert!(copy.shown && copy.slots == [["pen"]]);
        assert_ne!(copy.pos, find(&app.ui, "Pens").unwrap().pos, "not on top of the toolbar it copies");
        assert_ne!(copy.pos, find(&app.ui, "Pens Copy 2").unwrap().pos, "nor on top of an earlier copy");
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
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        let mut ui = prefs.sanitized();
        crate::workspaces::switch(&mut ui, "Essentials").unwrap();
        crate::workspaces::switch(&mut ui, "Drawing").unwrap();
        assert_eq!(ui.custom_toolbars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Pens", "Other", "Pens Copy"]);
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

    /// The Manage Toolbars dialog: the name typed for the selected toolbar renames it at OK, and a
    /// name another toolbar has keeps the dialog open.
    #[test]
    fn the_manage_toolbars_dialog_renames_the_selected_toolbar() {
        let mut app = app();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        assert_eq!(app.run("window.toolbar.manage", json!({})).unwrap(), Value::Null);
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.kind, MANAGE);
        d.fields.insert("selected".into(), json!("Mine"));
        d.fields.insert("name".into(), json!("Pens"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert!(find(&app.ui, "Pens").is_some() && find(&app.ui, "Mine").is_none());
        // A name taken by another toolbar keeps the dialog open.
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        app.run("window.toolbar.manage", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("selected".into(), json!("Pens"));
        d.fields.insert("name".into(), json!("other"));
        assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains("already"));
        assert!(app.ui.dialog.is_some());
        // Drawn with no toolbar selected, the dialog stays open.
        let ctx = egui::Context::default();
        app.ui.dialog.as_mut().unwrap().fields.insert("selected".into(), json!(""));
        for k in 0..3 {
            window_frame(&mut app, &ctx, f64::from(k) * 0.05, 800.0);
        }
        assert!(app.ui.dialog.as_ref().is_some_and(|d| d.kind == MANAGE));
    }

    /// An open flyout of a group on a toolbar closes when that toolbar is renamed or deleted; the
    /// flyout of another toolbar or of the Tools panel stays open.
    #[test]
    fn renaming_or_deleting_a_toolbar_closes_only_its_own_flyout() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        let mut time = 0.0;
        let mut frames = |app: &mut VectorcraftApp| {
            for _ in 0..2 {
                time += 0.05;
                window_frame(app, &ctx, time, 800.0);
            }
        };
        frames(&mut app);
        let open = |app: &mut VectorcraftApp, toolbar: Option<&str>| {
            app.ui.flyout = Some(0);
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::unique("flyout-tools"), vec!["rectangle".to_string(), "ellipse".to_string()]);
                d.insert_temp(crate::toolbar::flyout_toolbar_id(), toolbar.map(String::from));
            });
        };
        for (toolbar, change) in [
            (None, json!({"name": "Other", "to": "Second"})),
            (Some("Mine"), json!({"name": "Second", "to": "Third"})),
            (Some("Mine"), json!({"name": "Mine", "to": "mine"})),
        ] {
            open(&mut app, toolbar);
            app.run("window.toolbar.rename", change).unwrap();
            frames(&mut app);
            assert_eq!(app.ui.flyout, Some(0), "{toolbar:?}'s flyout stays open");
        }
        open(&mut app, Some("mine"));
        app.run("window.toolbar.rename", json!({"name": "mine", "to": "Pens"})).unwrap();
        frames(&mut app);
        assert_eq!(app.ui.flyout, None, "the renamed toolbar's flyout closes");
        open(&mut app, Some("Pens"));
        app.run("window.toolbar.delete", json!({"name": "Third"})).unwrap();
        frames(&mut app);
        assert_eq!(app.ui.flyout, Some(0), "another toolbar's delete leaves it open");
        app.run("window.toolbar.delete", json!({"name": "Pens"})).unwrap();
        frames(&mut app);
        assert_eq!(app.ui.flyout, None, "the deleted toolbar's flyout closes");
        // Another toolbar that takes the old name before the next frame doesn't keep it open.
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        frames(&mut app);
        open(&mut app, Some("Mine"));
        app.run("window.toolbar.rename", json!({"name": "Mine", "to": "X"})).unwrap();
        app.run("window.toolbar.rename", json!({"name": "Other", "to": "Mine"})).unwrap();
        assert_eq!(app.ui.flyout, None, "renamed, then its name taken");
        open(&mut app, Some("Mine"));
        app.run("window.toolbar.delete", json!({"name": "Mine"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        assert_eq!(app.ui.flyout, None, "deleted, then its name taken");
    }

    /// Deleting or renaming a toolbar while its title bar is dragged ends the drag: Escape then
    /// leaves a toolbar that took its name where it is. The drag of another toolbar goes on, and so
    /// does the drag of a toolbar whose new name changes only the case.
    #[test]
    fn deleting_or_renaming_the_toolbar_being_dragged_ends_its_drag() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        // A drag of the toolbar's title bar in progress, begun where the toolbar is now.
        let hold = |app: &VectorcraftApp, name: &str| {
            let bar = find(&app.ui, name).unwrap();
            let m = Move {
                name: bar.name.clone(),
                from: pos2(10.0, 10.0),
                grab: egui::Vec2::ZERO,
                dock: bar.dock,
                right: bar.dock_right,
                pos: bar.pos,
                cancelled: false,
            };
            ctx.data_mut(|d| d.insert_temp(move_id(), m));
        };
        let held = || ctx.data(|d| d.get_temp::<Move>(move_id())).is_some();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        app.run("window.toolbar.dock", json!({"name": "Mine", "side": "right"})).unwrap();
        hold(&app, "Mine");
        delete(&mut app.ui, Some(&ctx), "Other").unwrap();
        assert!(held(), "another toolbar's delete leaves the drag going");
        delete(&mut app.ui, Some(&ctx), "Mine").unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        let at = find(&app.ui, "Mine").unwrap().pos;
        assert!(!cancel(&mut app.ui, &ctx), "the deleted toolbar's drag ended");
        assert_eq!((place(&app, "Mine"), find(&app.ui, "Mine").unwrap().pos), ((None, false), at), "deleted, then its name taken");
        app.run("window.toolbar.dock", json!({"name": "Mine", "side": "right"})).unwrap();
        hold(&app, "Mine");
        rename(&mut app.ui, Some(&ctx), "Mine", "mine").unwrap();
        assert!(held(), "a new name that changes only the case leaves the drag going");
        rename(&mut app.ui, Some(&ctx), "mine", "Pens").unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        let at = find(&app.ui, "Mine").unwrap().pos;
        assert!(!cancel(&mut app.ui, &ctx), "the renamed toolbar's drag ended");
        assert_eq!((place(&app, "Mine"), find(&app.ui, "Mine").unwrap().pos), ((None, false), at), "renamed, then its name taken");
        assert_eq!(place(&app, "Pens"), (Some(0), true), "the renamed toolbar stays docked");
    }

    /// A toolbar deleted while Manage Toolbars has it selected leaves none selected, and a click
    /// on + or the trash can meant for it does nothing, so the dialog can go on.
    #[test]
    fn manage_toolbars_lets_go_of_a_toolbar_deleted_elsewhere() {
        let mut app = app();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        for action in [Some(false), Some(true), None] {
            app.run("window.toolbar.manage", json!({})).unwrap();
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("selected".into(), json!("Mine"));
            d.fields.insert("name".into(), json!("Typed"));
            app.run("window.toolbar.delete", json!({"name": "Mine"})).unwrap();
            manage_step(&mut app, action.is_none(), None, action);
            let d = app.ui.dialog.as_ref().unwrap();
            assert_eq!((d.str("selected"), d.str("name")), (String::new(), String::new()), "{action:?}");
            assert_eq!(listed(&app), ["Other"], "{action:?}");
            app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
            app.run("window.toolbar.delete", json!({"name": "Other"})).unwrap();
            app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        }
        // OK closes the dialog.
        app.ui.dialog.as_mut().unwrap().fields.insert("selected".into(), json!("Gone"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
    }

    /// A name that can't be given to the selected toolbar stops a click on +, the trash can or
    /// another toolbar, so the name typed stays to be corrected.
    #[test]
    fn a_refused_name_stops_the_next_click_in_manage_toolbars() {
        let mut app = app();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        app.run("window.toolbar.manage", json!({})).unwrap();
        let typed = |app: &mut VectorcraftApp| {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("selected".into(), json!("Mine"));
            d.fields.insert("name".into(), json!("other"));
        };
        for (pick, action) in [(None, Some(false)), (None, Some(true)), (Some("Other".to_string()), None)] {
            typed(&mut app);
            manage_step(&mut app, false, pick, action);
            assert_eq!(listed(&app), ["Mine", "Other"]);
            let d = app.ui.dialog.as_ref().unwrap();
            assert_eq!((d.str("selected"), d.str("name")), ("Mine".to_string(), "other".to_string()));
        }
        // A name that is free renames the toolbar first, then the trash can deletes it.
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Pens"));
        manage_step(&mut app, false, None, Some(false));
        assert_eq!(listed(&app), ["Other"]);
    }

    /// A new name and a copy's name keep to the length New Toolbar allows, so the toolbar is kept
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

    /// A copy made while Tab hides the toolbars shows alone, as a new toolbar does.
    #[test]
    fn a_copy_made_while_tab_hides_the_toolbars_shows_alone() {
        let mut app = app();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Other"})).unwrap();
        app.ui.tab_hid = Some(Default::default());
        app.run("window.toolbar.duplicate", json!({"name": "Mine"})).unwrap();
        assert!(app.ui.tab_hid.is_none());
        let shown: Vec<_> = app.ui.custom_toolbars.iter().filter(|b| b.shown).map(|b| b.name.as_str()).collect();
        assert_eq!(shown, ["Mine Copy"]);
    }

    /// A hand-edited file: unnamed toolbars and repeated names go, unknown tools go, each tool once,
    /// and a group shows a tool chosen last only when the group holds it.
    #[test]
    fn toolbars_load_sanitized() {
        let bar = |name: &str, slots: &[&[&str]]| CustomToolbar {
            name: name.into(),
            slots: slots.iter().map(|s| s.iter().map(|id| id.to_string()).collect()).collect(),
            shown: true,
            pos: [f32::NAN, 0.0],
            dock: None,
            dock_right: false,
            slot_tool: Default::default(),
            double: false,
            floating: ["pen", "nope", "pen"]
                .iter()
                .map(|id| crate::state::FloatingFlyout { tools: vec![id.to_string(), "curvature".into()], pos: [0.0, 0.0] })
                .collect(),
        };
        let long = "x".repeat(MAX_NAME + 1);
        let mut bars = vec![
            bar("A", &[&["pen", "nope", "curvature"], &["pen"], &["nope"]]),
            bar("a", &[]),
            bar(" ", &[]),
            bar(&long, &[]),
            bar("B", &[]),
            bar("C", &[&["pen"]]),
        ];
        // The tool a group shows: kept only for a group the toolbar holds, and a tool in it.
        let chosen = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        bars[0].slot_tool = chosen(&[("pen", "curvature"), ("curvature", "pen"), ("nope", "pen")]);
        bars[4].slot_tool = chosen(&[("pen", "curvature")]);
        bars[5].slot_tool = chosen(&[("pen", "curvature")]);
        // Docked places with gaps and repeats, and one dropped with its toolbar, are numbered from 0.
        (bars[0].dock, bars[1].dock, bars[4].dock, bars[5].dock) = (Some(7), Some(0), Some(7), Some(3));
        sanitize(&mut bars);
        assert_eq!(bars.iter().map(|b| b.dock).collect::<Vec<_>>(), [Some(1), Some(2), Some(0)]);
        assert_eq!(bars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["A", "B", "C"], "a name past {MAX_NAME} characters goes too");
        assert_eq!(bars[0].slot_tool, chosen(&[("pen", "curvature")]));
        assert!(bars[1].slot_tool.is_empty() && bars[2].slot_tool.is_empty(), "{:?} {:?}", bars[1].slot_tool, bars[2].slot_tool);
        assert_eq!(bars[0].slots, [["pen", "curvature"]]);
        assert_eq!(bars[0].pos, NEW_AT);
        // One strip per group, of known tools, for a group the toolbar holds.
        assert_eq!(bars[0].floating.iter().map(|f| f.tools.join(",")).collect::<Vec<_>>(), ["pen,curvature"]);
        assert!(bars[1].floating.is_empty(), "B holds no group");
    }

    /// Where the toolbar named `name` is: its place in its edge's stack (none: floating) and
    /// whether that edge is the right one.
    fn place(app: &VectorcraftApp, name: &str) -> (Option<u32>, bool) {
        let b = find(&app.ui, name).unwrap();
        (b.dock, b.dock_right)
    }

    /// `window.toolbar.dock` docks a toolbar at an edge, at the end of its stack or at the place
    /// asked for, and `window.toolbar.float` floats it where it floated last or at x, y; bad params
    /// change nothing. Hiding and renaming keep its place, a copy floats, and deleting closes the
    /// stack up. The places are listed, and kept with the workspace and the preferences.
    #[test]
    fn toolbars_dock_and_float_by_their_commands() {
        let mut app = app();
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        for name in ["A", "B", "C"] {
            app.run("window.toolbar.new", json!({ "name": name })).unwrap();
        }
        assert_eq!(place(&app, "A"), (None, false), "New Toolbar floats");
        let a_pos = find(&app.ui, "A").unwrap().pos;
        let r = app.run("window.toolbar.dock", json!({"name": "a"})).unwrap();
        assert_eq!(r, json!({"toolbar": "A", "floating": false, "side": "left", "order": 0, "pos": a_pos}));
        app.run("window.toolbar.dock", json!({"name": "B", "side": "left"})).unwrap();
        app.run("window.toolbar.dock", json!({"name": "C", "side": "start", "order": 0})).unwrap();
        assert_eq!([place(&app, "C"), place(&app, "A"), place(&app, "B")], [(Some(0), false), (Some(1), false), (Some(2), false)]);
        // Docked again at the edge it is docked at, it stays where it is.
        app.run("window.toolbar.dock", json!({"name": "A"})).unwrap();
        assert_eq!(place(&app, "A"), (Some(1), false));
        app.run("window.toolbar.dock", json!({"name": "A", "order": 2})).unwrap();
        assert_eq!([place(&app, "C"), place(&app, "B"), place(&app, "A")], [(Some(0), false), (Some(1), false), (Some(2), false)]);
        // Bad params are errors and change nothing.
        let before = app.ui.custom_toolbars.clone();
        for p in [
            json!({"name": "A", "side": "top"}),
            json!({"name": "A", "side": 1}),
            json!({"name": "A", "order": 3}),
            json!({"name": "A", "order": -1}),
            json!({"name": "A", "order": 0.5}),
            json!({"name": "A", "order": "1"}),
            json!({"name": "Nope"}),
            json!({}),
        ] {
            assert!(app.run("window.toolbar.dock", p.clone()).is_err(), "{p}");
        }
        for p in
            [json!({"name": "A", "x": 10}), json!({"name": "A", "x": 1e7, "y": 0}), json!({"name": "A", "x": "1", "y": 2}), json!({"name": "Nope"})]
        {
            assert!(app.run("window.toolbar.float", p.clone()).is_err(), "{p}");
        }
        assert_eq!(app.ui.custom_toolbars, before);
        // At the right edge; the left edge's stack closes up.
        let r = app.run("window.toolbar.dock", json!({"name": "A", "side": "RIGHT"})).unwrap();
        assert_eq!((&r["side"], &r["order"]), (&json!("right"), &json!(0)));
        assert_eq!([place(&app, "C"), place(&app, "B")], [(Some(0), false), (Some(1), false)]);
        // Floating where it floated last or at x, y; docked again at the edge it docked at last.
        let r = app.run("window.toolbar.float", json!({"name": "A"})).unwrap();
        assert_eq!(r, json!({"toolbar": "A", "floating": true, "side": "right", "order": null, "pos": a_pos}));
        app.run("window.toolbar.float", json!({"name": "A", "x": 300, "y": 200})).unwrap();
        assert_eq!(find(&app.ui, "A").unwrap().pos, [300.0, 200.0]);
        assert_eq!(app.run("window.toolbar.dock", json!({"name": "A"})).unwrap()["side"], json!("right"));
        // Hidden and renamed, it keeps its place; a copy floats; deleted, the stack closes up.
        app.run("window.toolbar.custom", json!({"name": "C", "show": false})).unwrap();
        app.run("window.toolbar.rename", json!({"name": "C", "to": "Cee"})).unwrap();
        assert_eq!(place(&app, "Cee"), (Some(0), false));
        let copy = app.run("window.toolbar.duplicate", json!({"name": "B"})).unwrap();
        assert_eq!(place(&app, copy["toolbar"].as_str().unwrap()), (None, false));
        app.run("window.toolbar.delete", json!({"name": "Cee"})).unwrap();
        assert_eq!(place(&app, "B"), (Some(0), false));
        let list = app.run("window.toolbar.list", json!({})).unwrap();
        let rows: Vec<_> = list.as_array().unwrap().iter().map(|b| json!([b["name"], b["floating"], b["side"], b["order"]])).collect();
        assert_eq!(rows, [json!(["A", false, "right", 0]), json!(["B", false, "left", 0]), json!(["B Copy", true, "left", null])]);
        // Kept with the workspace and the preferences.
        let kept = app.ui.custom_toolbars.clone();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        app.run("window.workspace", json!({"name": "Drawing"})).unwrap();
        assert_eq!(app.ui.custom_toolbars, kept);
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        assert_eq!(prefs.sanitized().custom_toolbars, kept);
        // Saved before toolbars docked, a toolbar floats.
        let mut old = serde_json::to_value(&kept[0]).unwrap();
        let fields = old.as_object_mut().unwrap();
        assert!(fields.remove("dock").is_some() && fields.remove("dock_right").is_some());
        let old: CustomToolbar = serde_json::from_value(old).unwrap();
        assert_eq!((old.dock, old.dock_right), (None, false));
    }

    /// Docked toolbars show in lanes along the window's edges, the left one right of the Tools
    /// panel: stacked from the top, centered, each in its share of the lane, the lane as wide as its
    /// widest toolbar and gone while none shows. The canvas and the panels take the space left.
    #[test]
    fn docked_toolbars_stack_in_lanes_along_the_window_edges() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        for name in ["A", "B", "R"] {
            app.run("window.toolbar.new", json!({ "name": name })).unwrap();
        }
        for tool in vectorcraft_tools::TOOL_GROUPS.iter().flat_map(|g| g.iter()).take(30) {
            app.run("toolbar.addTool", json!({"tool": tool.id, "toolbar": "A"})).unwrap();
        }
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "B"})).unwrap();
        let mut time = 0.0;
        let mut frames = |app: &mut VectorcraftApp| {
            for _ in 0..4 {
                time += 0.05;
                window_frame(app, &ctx, time, 600.0);
            }
        };
        frames(&mut app);
        let rect = |name: &str| ctx.memory(|m| m.area_rect(area_id(name))).unwrap();
        let lane = |right: bool| ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(right)));
        assert!(lane(false).is_none() && lane(true).is_none(), "no lane while every toolbar floats");
        for (name, side) in [("A", "left"), ("B", "left"), ("R", "right")] {
            app.run("window.toolbar.dock", json!({ "name": name, "side": side })).unwrap();
        }
        frames(&mut app);
        let tools = ctx.data(|d| d.get_temp::<egui::Rect>(crate::toolbar::docked_rect_id())).unwrap();
        let (left, right) = (lane(false).unwrap(), lane(true).unwrap());
        let (a, b, r) = (rect("A"), rect("B"), rect("R"));
        assert!((left.left() - tools.right()).abs() < 1.0 && (left.width() - WIDTH - 2.0 * LANE_GAP).abs() < 0.5, "{left:?} {tools:?}");
        assert!(left.contains_rect(a) && left.contains_rect(b) && right.contains_rect(r), "{left:?} {a:?} {b:?} {right:?} {r:?}");
        assert!((right.right() - 1200.0).abs() < 0.5, "{right:?}");
        assert!(b.top() >= a.bottom() && (a.center().x - left.center().x).abs() < 1.0 && (b.center().x - a.center().x).abs() < 1.0, "{a:?} {b:?}");
        // A's 30 tools scroll in its half of the lane.
        assert!(a.height() <= left.height() / 2.0, "{a:?} {left:?}");
        // The canvas ends left of the right lane.
        let canvas_right = ctx.data(|d| d.get_temp::<egui::Rect>(crate::tools_float::tools_zone_right_id())).unwrap().right();
        assert!(canvas_right <= right.left() + 0.5, "{canvas_right} {right:?}");
        // Two columns widen the lane, and the toolbar stays docked.
        app.run("window.toolbarColumns", json!({"toolbar": "B", "double": true})).unwrap();
        frames(&mut app);
        assert!((lane(false).unwrap().width() - WIDTH - BUTTON - 2.0 * LANE_GAP).abs() < 0.5);
        assert!(lane(false).unwrap().contains_rect(rect("B")) && place(&app, "B") == (Some(1), false));
        // Hidden, it keeps its place, and an empty lane goes.
        app.run("window.toolbar.custom", json!({"name": "R", "show": false})).unwrap();
        frames(&mut app);
        assert!(lane(true).is_none() && !drawn(&ctx, area_id("R"), egui::Order::Middle) && place(&app, "R") == (Some(0), true));
        // Tab hides the lanes with the toolbars.
        app.run("window.dock", json!({})).unwrap();
        frames(&mut app);
        assert!(lane(false).is_none() && !drawn(&ctx, area_id("A"), egui::Order::Middle));
    }

    /// With Keep Tools Beside the Canvas on, the left lane stands right of Tools, which stands right
    /// of the panels docked on the left, and its target covers Tools and the lane, not the panels. In a
    /// narrow window the lane leaves the canvas its least width of 200 points. With the setting off
    /// again, the lane goes back to the window's edge, right of Tools.
    #[test]
    fn the_left_lane_follows_tools_docked_beside_the_canvas() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "A"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "A"})).unwrap();
        app.run("window.panel.dock", json!({"panel": "layers", "side": "Start"})).unwrap();
        app.run("window.toolsBesideCanvas", json!({"on": true})).unwrap();
        app.run("window.toolbar.dock", json!({"name": "A", "side": "left"})).unwrap();
        let mut time = 0.0;
        let mut frames = |app: &mut VectorcraftApp, size: egui::Vec2| {
            for _ in 0..6 {
                time += 0.05;
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                let raw = egui::RawInput { time: Some(time), screen_rect: Some(screen), ..Default::default() };
                let mut out = ctx.run_ui(raw, |ui| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                });
                out.textures_delta.clear();
            }
        };
        frames(&mut app, vec2(1200.0, 600.0));
        let tools = || ctx.data(|d| d.get_temp::<egui::Rect>(crate::toolbar::docked_rect_id())).unwrap();
        let lane = || ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(false))).unwrap();
        let zone = ctx.data(|d| d.get_temp::<egui::Rect>(zone_id(false))).unwrap();
        let a = ctx.memory(|m| m.area_rect(area_id("A"))).unwrap();
        assert!(tools().left() > 100.0, "Tools stands right of the left dock: {:?}", tools());
        assert!((lane().left() - tools().right()).abs() < 1.0, "{:?} {:?}", lane(), tools());
        assert!(lane().contains_rect(a), "{a:?} {:?}", lane());
        assert!(zone.contains(tools().center()) && zone.right() >= lane().right() - 0.5, "{zone:?}");
        assert!(!zone.contains(pos2(4.0, tools().center().y)), "the left dock is not a target: {zone:?}");
        // In a narrow, short window, a stack that wraps leaves the canvas its 200 points.
        for k in 0..12 {
            let name = format!("T{k}");
            app.run("window.toolbar.new", json!({ "name": name })).unwrap();
            app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": name})).unwrap();
            app.run("window.toolbar.dock", json!({ "name": name, "side": "left" })).unwrap();
        }
        frames(&mut app, vec2(700.0, 320.0));
        // The canvas panel's right edge, and the canvas drawn right of the lane.
        let right = ctx.data(|d| d.get_temp::<egui::Rect>(crate::tools_float::tools_zone_right_id())).unwrap().right();
        let canvas = app.canvas_rect.unwrap();
        assert!(right - lane().right() >= 199.5 && canvas.left() >= lane().right() - 0.5, "{right} {canvas:?} {:?} {:?}", lane(), tools());
        app.run("window.toolsBesideCanvas", json!({"on": false})).unwrap();
        frames(&mut app, vec2(1200.0, 600.0));
        assert!(tools().left() < 1.0 && (lane().left() - tools().right()).abs() < 1.0, "{:?} {:?}", lane(), tools());
    }

    /// More docked toolbars than a short lane holds in one column wrap into more columns: each stays
    /// inside the lane with its title bar, and none covers another. With more than half the window's
    /// width of columns needed (60 toolbars), the lane stops at half the width and the toolbars show
    /// only their title bars, still each in view.
    #[test]
    fn a_short_lane_keeps_every_docked_toolbar_inside_it() {
        for count in [12, 60] {
            let mut app = app();
            app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
            let ctx = egui::Context::default();
            let names: Vec<String> = (0..count).map(|k| format!("T{k}")).collect();
            for name in &names {
                app.run("window.toolbar.new", json!({ "name": name })).unwrap();
                app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": name})).unwrap();
                app.run("window.toolbar.dock", json!({ "name": name, "side": "right" })).unwrap();
            }
            for k in 0..6 {
                window_frame(&mut app, &ctx, 0.05 * f64::from(k + 1), 320.0);
            }
            let lane = ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(true))).unwrap();
            let rects: Vec<egui::Rect> = names.iter().map(|name| ctx.memory(|m| m.area_rect(area_id(name))).unwrap()).collect();
            for (name, r) in names.iter().zip(&rects) {
                assert!(lane.expand(0.5).contains_rect(*r), "{count}: {name} {r:?} {lane:?}");
                assert!(r.height() >= GRIP, "{count}: {name} {r:?}");
            }
            for (i, a) in rects.iter().enumerate() {
                for (j, b) in rects.iter().enumerate().skip(i + 1) {
                    assert!(a.intersect(*b).area() < 1.0, "{count}: {} {a:?} covers {} {b:?}", names[i], names[j]);
                }
            }
            assert!(lane.width() > WIDTH + 2.0 * LANE_GAP + 1.0, "the stack wraps into more columns: {lane:?}");
            assert!(lane.width() <= 600.0, "{count}: the lane leaves the canvas at least half the window: {lane:?}");
            assert!((lane.right() - 1200.0).abs() < 0.5, "{lane:?}");
            let bare = rects.iter().all(|r| r.height() < DOCKED_LEAST - 1.0);
            assert_eq!(bare, count == 60, "{count}: title bars alone only when the columns run out: {rects:?}");
        }
    }

    /// A lone docked toolbar in a lane too short for its tools shows only its title bar, inside the
    /// lane.
    #[test]
    fn a_lone_toolbar_in_a_short_lane_stays_inside_it() {
        let mut short = 0;
        for height in (120..=240).step_by(10) {
            let mut app = app();
            app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
            let ctx = egui::Context::default();
            app.run("window.toolbar.new", json!({"name": "A"})).unwrap();
            app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "A"})).unwrap();
            app.run("window.toolbar.dock", json!({"name": "A", "side": "right"})).unwrap();
            for k in 0..6 {
                window_frame(&mut app, &ctx, 0.05 * f64::from(k + 1), height as f32);
            }
            let lane = ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(true))).unwrap();
            if lane.height() >= DOCKED_LEAST + 2.0 * LANE_GAP {
                continue;
            }
            short += 1;
            let a = ctx.memory(|m| m.area_rect(area_id("A"))).unwrap();
            assert!(lane.expand(0.5).contains_rect(a), "{height}: {a:?} {lane:?}");
            assert!(a.height() >= GRIP, "{height}: the title bar shows: {a:?}");
        }
        assert!(short > 0, "some window heights leave a lane shorter than one toolbar");
    }

    /// A column too short for even its toolbars' title bars shows as many as fit, none covering
    /// another, and the mouse wheel over it scrolls through the rest, one toolbar a notch.
    #[test]
    fn a_column_too_short_for_its_title_bars_scrolls() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        let names: Vec<String> = (0..64).map(|k| format!("T{k}")).collect();
        for name in &names {
            app.run("window.toolbar.new", json!({ "name": name })).unwrap();
            app.run("window.toolbar.dock", json!({ "name": name, "side": "right" })).unwrap();
        }
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.05;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 200.0));
            let raw = egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() };
            ctx.run_ui(raw, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            })
            .textures_delta
            .clear();
        };
        for _ in 0..6 {
            frame(&mut app, vec![]);
        }
        let shown =
            |ctx: &egui::Context| -> Vec<usize> { (0..names.len()).filter(|&k| drawn(ctx, area_id(&names[k]), egui::Order::Middle)).collect() };
        let lane = ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(true))).unwrap();
        let before = shown(&ctx);
        assert!(!before.is_empty() && before.len() < names.len(), "{before:?} {lane:?}");
        let rects: Vec<egui::Rect> = before.iter().map(|&k| ctx.memory(|m| m.area_rect(area_id(&names[k]))).unwrap()).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(lane.expand(0.5).contains_rect(*a), "{a:?} {lane:?}");
            for b in rects.iter().skip(i + 1) {
                assert!(a.intersect(*b).area() < 1.0, "{a:?} covers {b:?}");
            }
        }
        assert!(before.contains(&0));
        // The wheel over the first column shows the next toolbar of that column.
        let at = pos2(lane.left() + LANE_GAP + WIDTH / 2.0, lane.center().y);
        let wheel = |dy: f32| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, dy),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        };
        frame(&mut app, vec![egui::Event::PointerMoved(at), wheel(-1.0)]);
        frame(&mut app, vec![]);
        let after = shown(&ctx);
        assert!(!after.contains(&0) && after.len() == before.len(), "{before:?} {after:?}");
        frame(&mut app, vec![egui::Event::PointerMoved(at), wheel(1.0)]);
        frame(&mut app, vec![]);
        assert_eq!(shown(&ctx), before, "the wheel back shows the first again");
    }

    /// A lane holds one column while each docked toolbar gets [`DOCKED_LEAST`] points, and as few
    /// more columns as give each that much, the toolbars spread evenly over them, up to the most
    /// columns the lane may take.
    #[test]
    fn short_lanes_wrap_their_stack_into_columns() {
        let tall = 2.0 * (DOCKED_LEAST + LANE_GAP) + LANE_GAP;
        assert_eq!(lane_columns(0, 500.0, 8), (1, 1));
        assert_eq!(lane_columns(2, tall, 8), (1, 2));
        assert_eq!(lane_columns(3, tall, 8), (2, 2));
        assert_eq!(lane_columns(5, tall, 8), (3, 2));
        assert_eq!(lane_columns(4, 10.0, 8), (4, 1), "a lane shorter than one toolbar puts each in its own column");
        assert_eq!(lane_columns(3, f32::NAN, 8), (3, 1));
        assert_eq!(lane_columns(9, tall, 2), (2, 5), "at most the columns allowed");
        assert_eq!(lane_columns(9, tall, 0), (1, 9));
        assert_eq!(most_columns(3.0 * WIDTH + 4.0 * LANE_GAP, WIDTH), 3);
        assert_eq!(most_columns(10.0, WIDTH), 1);
        assert_eq!(most_columns(f32::NAN, WIDTH), 1);
    }

    /// The frame that docks a toolbar lays its lane out after the toolbars are drawn, so it asks for
    /// the next frame at once, which draws the toolbar in the lane, without other input.
    #[test]
    fn a_new_lane_draws_its_toolbar_in_the_next_frame() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp| {
            time += 0.05;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 600.0));
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(screen), ..Default::default() }, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
            out.viewport_output.values().map(|v| v.repaint_delay).min().unwrap_or(std::time::Duration::MAX)
        };
        for _ in 0..4 {
            frame(&mut app);
        }
        app.run("window.toolbar.dock", json!({"name": "Mine"})).unwrap();
        assert_eq!(frame(&mut app), std::time::Duration::ZERO, "the frame that lays the new lane out asks for another");
        frame(&mut app);
        let lane = ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(false))).unwrap();
        assert!(lane.contains_rect(ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap()));
    }

    /// A docked toolbar dragged inside its lane stays docked. A toolbar's title bar dropped on the
    /// window's edge docks it there; dragged out of its lane,
    /// it floats under the pointer; Escape during the drag puts it back; a click near the edge does
    /// not dock it. The title bar's context menu docks and floats it.
    #[test]
    fn a_title_bar_dropped_on_an_edge_docks_the_toolbar() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.05;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0));
            let raw = egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
            // The texts drawn, with where each is.
            let texts = out.shapes.iter().filter_map(|c| {
                if let egui::Shape::Text(t) = &c.shape { Some((t.galley.text().to_string(), t.visual_bounding_rect())) } else { None }
            });
            texts.collect::<Vec<_>>()
        };
        let button = |pos, pressed, button| egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
        let primary = egui::PointerButton::Primary;
        let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        // Docked and dragged down its lane, past its own bottom, it stays docked.
        app.run("window.toolbar.dock", json!({"name": "Mine", "side": "left"})).unwrap();
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let r = ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap();
        let lane = ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(false))).unwrap();
        let (from, below) = (pos2(r.center().x, r.top() + 1.0 + GRIP / 2.0), pos2(lane.center().x, lane.bottom() - 20.0));
        assert!(!r.contains(below), "{r:?} {lane:?}");
        frame(&mut app, vec![egui::Event::PointerMoved(from), button(from, true, primary)]);
        for k in 1..=6 {
            frame(&mut app, vec![egui::Event::PointerMoved(from + (below - from) * (k as f32 / 6.0))]);
            assert_eq!(place(&app, "Mine"), (Some(0), false), "a drag inside the lane keeps it docked, step {k}");
        }
        frame(&mut app, vec![button(below, false, primary)]);
        app.run("window.toolbar.float", json!({"name": "Mine"})).unwrap();
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        // A press on the title bar's grip, moved in steps to `to` (`cut` after the step it names),
        // then released there.
        let mut drag = |app: &mut VectorcraftApp, from: egui::Pos2, to: egui::Pos2, cut: Option<usize>| {
            frame(app, vec![egui::Event::PointerMoved(from), button(from, true, primary)]);
            for k in 1..=6 {
                let mut events = vec![egui::Event::PointerMoved(from + (to - from) * (k as f32 / 6.0))];
                if cut == Some(k) {
                    events.push(escape.clone());
                }
                frame(app, events);
            }
            frame(app, vec![button(to, false, primary)]);
            frame(app, vec![]);
            frame(app, vec![]);
        };
        let grip = |ctx: &egui::Context| {
            let r = ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap();
            pos2(r.center().x, r.top() + 1.0 + GRIP / 2.0)
        };
        // Dropped on the window's left edge, it docks there.
        drag(&mut app, grip(&ctx), pos2(20.0, 400.0), None);
        assert_eq!(place(&app, "Mine"), (Some(0), false));
        let lane = ctx.data(|d| d.get_temp::<egui::Rect>(lane_id(false))).unwrap();
        assert!(lane.contains_rect(ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap()));
        // Escape while it is dragged out of the lane puts it back in its place.
        drag(&mut app, grip(&ctx), pos2(600.0, 300.0), Some(4));
        assert_eq!(place(&app, "Mine"), (Some(0), false), "Escape keeps it docked");
        // Escape in the frame of the first move after the press: no drag starts.
        drag(&mut app, grip(&ctx), pos2(600.0, 300.0), Some(1));
        assert_eq!(place(&app, "Mine"), (Some(0), false), "Escape with the first move keeps it docked");
        // Dragged out of the lane, it floats under the pointer.
        drag(&mut app, grip(&ctx), pos2(600.0, 300.0), None);
        assert_eq!(place(&app, "Mine"), (None, false));
        assert!(ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap().contains(pos2(600.0, 300.0)));
        // Escape while it is dragged to the right edge leaves it floating where it was.
        let at = find(&app.ui, "Mine").unwrap().pos;
        drag(&mut app, grip(&ctx), pos2(1195.0, 400.0), Some(5));
        assert_eq!((place(&app, "Mine"), find(&app.ui, "Mine").unwrap().pos), ((None, false), at));
        drag(&mut app, grip(&ctx), pos2(1195.0, 400.0), Some(1));
        assert_eq!((place(&app, "Mine"), find(&app.ui, "Mine").unwrap().pos), ((None, false), at), "Escape with the first move");
        // Dropped on the right edge, it docks there.
        drag(&mut app, grip(&ctx), pos2(1195.0, 400.0), None);
        assert_eq!(place(&app, "Mine"), (Some(0), true));
        // Floating over the edge, a click on its title bar does not dock it.
        app.run("window.toolbar.float", json!({"name": "Mine", "x": 2, "y": 300})).unwrap();
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        let at = grip(&ctx);
        frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true, primary)]);
        frame(&mut app, vec![button(at, false, primary)]);
        frame(&mut app, vec![]);
        assert_eq!(place(&app, "Mine"), (None, true));
        // The title bar's context menu: Dock to Left docks it, and Float Toolbar floats it again.
        for (item, after) in [("Dock to Left", (Some(0), false)), ("Float Toolbar", (None, false))] {
            let at = grip(&ctx);
            let secondary = egui::PointerButton::Secondary;
            frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true, secondary)]);
            frame(&mut app, vec![button(at, false, secondary)]);
            let texts = frame(&mut app, vec![]);
            let found = texts.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == item);
            let at = found.unwrap_or_else(|| panic!("no {item} in {texts:?}")).1.center();
            frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true, primary)]);
            frame(&mut app, vec![button(at, false, primary)]);
            frame(&mut app, vec![]);
            frame(&mut app, vec![]);
            assert_eq!(place(&app, "Mine"), after, "after {item}");
        }
        // Docked, and released in the frame the pointer leaves the lane, it floats there.
        app.run("window.toolbar.dock", json!({"name": "Mine"})).unwrap();
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        let at = grip(&ctx);
        frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true, primary)]);
        frame(&mut app, vec![egui::Event::PointerMoved(at + vec2(0.0, 8.0))]);
        assert_eq!(place(&app, "Mine"), (Some(0), false), "still in its lane");
        let out = pos2(600.0, 300.0);
        frame(&mut app, vec![egui::Event::PointerMoved(out), button(out, false, primary)]);
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        assert_eq!(place(&app, "Mine"), (None, false), "released outside its lane");
        assert!(ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap().contains(out));
    }
}
