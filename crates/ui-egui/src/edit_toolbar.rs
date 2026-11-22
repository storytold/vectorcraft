//! Edit Toolbar (#1018): the All Tools drawer that the ••• button at the bottom of the toolbar
//! opens beside it, and the toolbar edits it makes. The drawer lists every tool under the Basic
//! toolbar's group headings (and headings for the tools only the Advanced toolbar shows); the tools
//! already in the toolbar are dimmed. While it is open the toolbar can be edited: a tool dragged
//! from the drawer onto the toolbar is added there (dropped between two tools, or onto one to join
//! its group), one dragged from the toolbar into the drawer is removed, and the toolbar's tools
//! drag to new places. Escape or a click outside closes it. Picking a tool in it selects the tool.
//!
//! Behaviour from the public documentation: <https://helpx.adobe.com/illustrator/using/tools.html>
//! (Edit Toolbar, the All Tools drawer; tools are added, removed or rearranged only while the drawer
//! is open, and the ones already in the toolbar show dimmed in it).
//!
//! Each toolbar (Basic, Advanced) keeps its own edits in `UiState::toolbar_edits`, saved with the
//! preferences; `toolbar.addTool`, `toolbar.removeTool` and `toolbar.reset` make them. With their
//! `toolbar` param they edit a toolbar made with New Toolbar instead (`custom_toolbars`), whose •••
//! button opens the drawer beside it.

use std::collections::BTreeSet;
use std::sync::OnceLock;

// PLAN: assumed paths of the framework items under `craft_ui` / `craft_behavior`.
use craft::behavior::IconId;
use craft::behavior::overlay::Side;
use craft::ui::buttons::ToggleButton;
use craft::ui::contract::{Bind, Outcome as _, Show as _};
use craft::ui::menu::PanelRow;
use craft::ui::primitives::IconSpec;
use craft::ui::rails::{DrawerGroup, DropAt, ToolRailOutput, ToolSlot, ToolbarDrawer, ToolbarEdit};
use craft::ui::runtime::commands::CommandRef;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_tools::{TOOL_GROUPS, ToolInfo, tool_info};

use crate::VectorcraftApp;
use crate::panels::learn::locate::{self, Home};
use crate::state::{ToolbarCategory, ToolbarEdits};

/// Where a tool dropped on a toolbar slot goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// Its own slot, before the slot (on the divider above it, or left of it in two columns).
    Before,
    /// Its own slot, after the slot.
    After,
    /// Into the slot's group (its flyout), last.
    Group,
}

impl Place {
    const ALL: [Self; 3] = [Self::Before, Self::After, Self::Group];

    /// The `place` param (omitted: before).
    fn parse(p: &Value) -> Result<Self, String> {
        let key = "place";
        match p.get(key) {
            None | Some(Value::Null) => Ok(Self::Before),
            Some(v) => Self::ALL
                .into_iter()
                .find(|place| v.as_str() == Some(place.name()))
                .ok_or_else(|| format!("`{key}` must be one of {}", Self::ALL.map(Self::name).join(", "))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
            Self::Group => "group",
        }
    }
}

// ---------- the model ----------

/// A toolbar as it ships: the Basic toolbar's categories, or the Advanced toolbar's tool groups.
fn shipped(advanced: bool) -> Vec<ToolbarCategory> {
    let strings = |s: &[&str]| s.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    if advanced {
        let slots = TOOL_GROUPS.iter().map(|g| g.iter().map(|t| t.id.to_string()).collect()).collect();
        vec![ToolbarCategory { name: None, slots }]
    } else {
        crate::toolbar::BASIC
            .iter()
            .map(|(cat, slots)| ToolbarCategory { name: Some(cat.to_string()), slots: slots.iter().map(|s| strings(s)).collect() })
            .collect()
    }
}

fn current(edits: &mut ToolbarEdits, advanced: bool) -> &mut Option<Vec<ToolbarCategory>> {
    if advanced { &mut edits.advanced } else { &mut edits.basic }
}

/// The Basic toolbar category named `name`.
fn category(name: &str) -> Option<&'static str> {
    crate::toolbar::BASIC.iter().map(|(c, _)| *c).find(|c| *c == name)
}

/// The current toolbar's slots as Edit Toolbar left them, laid out as `toolbar::slots` lays them
/// out (the category name on each category's first slot); `None` while it is as it ships.
pub(crate) fn edited_slots(app: &VectorcraftApp) -> Option<Vec<(Option<&'static str>, Vec<&'static str>)>> {
    let edits = &app.ui.toolbar_edits;
    let cats = if app.ui.toolbar_advanced { &edits.advanced } else { &edits.basic };
    let mut out = vec![];
    for c in cats.as_ref()? {
        let mut name = c.name.as_deref().and_then(category);
        for s in &c.slots {
            let tools: Vec<&'static str> = s.iter().filter_map(|id| tool_info(id).map(|t| t.id)).collect();
            if !tools.is_empty() {
                out.push((name.take(), tools));
            }
        }
    }
    Some(out)
}

/// Drop the empty slots, then the empty categories.
fn prune(cats: &mut Vec<ToolbarCategory>) {
    for c in cats.iter_mut() {
        c.slots.retain(|s| !s.is_empty());
    }
    cats.retain(|c| !c.slots.is_empty());
}

/// Keep known tools, each once, and known category names (a hand-edited preferences file).
pub(crate) fn sanitize(edits: &mut ToolbarEdits) {
    for cats in [&mut edits.basic, &mut edits.advanced].into_iter().flatten() {
        let mut seen = BTreeSet::new();
        for c in cats.iter_mut() {
            c.name = c.name.as_deref().and_then(category).map(str::to_string);
            for s in &mut c.slots {
                s.retain(|id| tool_info(id).is_some() && seen.insert(id.clone()));
            }
        }
        prune(cats);
    }
}

/// Change the current toolbar with `f`: an edit that gives back the toolbar as it ships forgets
/// the edits. Returns the toolbar's slots.
fn edit(app: &mut VectorcraftApp, f: impl FnOnce(&mut Vec<ToolbarCategory>) -> Result<(), String>) -> Result<Value, String> {
    let advanced = app.ui.toolbar_advanced;
    let slot = current(&mut app.ui.toolbar_edits, advanced);
    let mut cats = slot.clone().unwrap_or_else(|| shipped(advanced));
    f(&mut cats)?;
    prune(&mut cats);
    *slot = (cats != shipped(advanced)).then_some(cats);
    Ok(slots_json(app))
}

/// Change the toolbar made with New Toolbar called `name` with `f`, and the groups torn off it with
/// it ([`crate::custom_toolbars::keep_strips`]). Returns its slots.
fn edit_custom(app: &mut VectorcraftApp, name: &str, f: impl FnOnce(&mut Vec<ToolbarCategory>) -> Result<(), String>) -> Result<Value, String> {
    let bar = crate::custom_toolbars::find_mut(&mut app.ui, name).ok_or_else(|| format!("no toolbar named `{name}`"))?;
    let mut cats = vec![ToolbarCategory { name: None, slots: bar.slots.clone() }];
    f(&mut cats)?;
    prune(&mut cats);
    bar.slots = cats.into_iter().flat_map(|c| c.slots).collect();
    let (id, slots) = (bar.id, json!(bar.slots));
    crate::custom_toolbars::keep_strips(app, id);
    Ok(slots)
}

/// Change the toolbar the `toolbar` param names (none: the current Basic or Advanced toolbar).
fn edit_target(app: &mut VectorcraftApp, p: &Value, f: impl FnOnce(&mut Vec<ToolbarCategory>, bool) -> Result<(), String>) -> Result<Value, String> {
    match crate::custom_toolbars::target(&app.ui, p)? {
        Some(name) => edit_custom(app, &name, |cats| f(cats, false)),
        None => {
            let named = !app.ui.toolbar_advanced;
            edit(app, |cats| f(cats, named))
        }
    }
}

/// The current toolbar's slots, each a list of tool ids (`toolbar.*` commands return it).
fn slots_json(app: &VectorcraftApp) -> Value {
    json!(crate::toolbar::slots(app).into_iter().map(|(_, s)| s).collect::<Vec<_>>())
}

/// The slots of the toolbar the drawer edits (one made with New Toolbar, or the Tools panel's), as
/// the `slot` param of `toolbar.addTool` counts them.
fn target_slots(app: &VectorcraftApp) -> Vec<Vec<&'static str>> {
    let slots: Vec<Vec<&'static str>> =
        match crate::custom_toolbars::drawer_target(&app.ui).and_then(|name| crate::custom_toolbars::find(&app.ui, name)) {
            Some(bar) => bar.slots.iter().map(|s| s.iter().filter_map(|id| tool_info(id).map(|t| t.id)).collect()).collect(),
            None => crate::toolbar::slots(app).into_iter().map(|(_, s)| s).collect(),
        };
    slots.into_iter().filter(|s| !s.is_empty()).collect()
}

/// Put `tool` by the slot numbered `slot` (over the toolbar's slots, 0 = the first; none or null:
/// at the end), moving it if it is in the toolbar already. A tool appended to a toolbar left without
/// categories starts one (`named`: under its own heading).
fn add(cats: &mut Vec<ToolbarCategory>, tool: &str, slot: Option<&Value>, place: Place, named: bool) -> Result<(), String> {
    let at: Vec<(usize, usize)> = cats.iter().enumerate().flat_map(|(c, cat)| (0..cat.slots.len()).map(move |s| (c, s))).collect();
    let bad_slot = || {
        let (key, min, max, x) = ("slot", 0, at.len().saturating_sub(1), slot.unwrap_or(&Value::Null));
        format!("`{key}` must be a whole number between {min} and {max} (got {x})")
    };
    let target = match slot {
        None | Some(Value::Null) => None,
        Some(v) => Some(*v.as_u64().and_then(|k| usize::try_from(k).ok()).and_then(|k| at.get(k)).ok_or_else(bad_slot)?),
    };
    let holds =
        |cats: &[ToolbarCategory], (c, s): (usize, usize)| cats.get(c).and_then(|cat| cat.slots.get(s)).is_some_and(|s| s.iter().any(|t| t == tool));
    if place == Place::Group && target.is_some_and(|t| holds(cats, t)) {
        // Onto its own group: nothing to do.
        return Ok(());
    }
    // Out of where it was, leaving its slot in place (emptied ones go after) so `target` holds.
    for c in cats.iter_mut() {
        for s in &mut c.slots {
            s.retain(|t| t != tool);
        }
    }
    let new = vec![tool.to_string()];
    let Some((c, s)) = target else {
        match cats.last_mut() {
            Some(cat) => cat.slots.push(new),
            None => cats.push(ToolbarCategory { name: named.then(|| heading(tool)).and_then(category).map(str::to_string), slots: vec![new] }),
        }
        return Ok(());
    };
    let cat = cats.get_mut(c).ok_or_else(bad_slot)?;
    match place {
        Place::Group => cat.slots.get_mut(s).ok_or_else(bad_slot)?.push(tool.to_string()),
        Place::Before => cat.slots.insert(s.min(cat.slots.len()), new),
        Place::After => cat.slots.insert((s + 1).min(cat.slots.len()), new),
    }
    Ok(())
}

/// Take `tool` out of the toolbar (`group`: with the rest of its group); one that isn't in it
/// stays out.
fn remove(cats: &mut [ToolbarCategory], tool: &str, group: bool) {
    if let Some(slot) = cats.iter_mut().flat_map(|c| c.slots.iter_mut()).find(|s| s.iter().any(|t| t == tool)) {
        if group {
            slot.clear();
        } else {
            slot.retain(|t| t != tool);
        }
    }
}

/// A known tool's id from the `tool` param.
fn tool_param(p: &Value) -> Result<&'static str, String> {
    let tool = p.get("tool").and_then(Value::as_str).ok_or("tool (a tool id) is required")?;
    tool_info(tool).map(|t| t.id).ok_or_else(|| format!("unknown tool `{tool}`"))
}

/// The Edit Toolbar commands; `None` for other ids.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "window.editToolbar" => (|| {
            let open = crate::menus::opt_bool(p, "open")?;
            let target = crate::custom_toolbars::target(&app.ui, p)?;
            // Toggled: open, or open beside another toolbar, closes or moves it.
            let open = open.unwrap_or(!(app.ui.tools_drawer && app.ui.drawer_toolbar == target));
            app.ui.tools_drawer = open;
            app.ui.drawer_toolbar = target.filter(|_| open);
            if open {
                // The toolbar the drawer opens beside shows.
                match app.ui.drawer_toolbar.clone() {
                    Some(name) => {
                        app.run("window.toolbar.custom", json!({ "name": name, "show": true }))?;
                    }
                    None => crate::toolbar::set_shown(app, true),
                }
            }
            Ok(json!(open))
        })(),
        "toolbar.addTool" => (|| {
            let tool = tool_param(p)?;
            let place = Place::parse(p)?;
            edit_target(app, p, |cats, named| add(cats, tool, p.get("slot"), place, named))
        })(),
        "toolbar.removeTool" => (|| {
            let tool = tool_param(p)?;
            let group = crate::menus::opt_bool(p, "group")?.unwrap_or(false);
            edit_target(app, p, |cats, _| {
                remove(cats, tool, group);
                Ok(())
            })
        })(),
        "toolbar.show" => (|| {
            let on = crate::menus::opt_bool(p, "on")?;
            if let Some(name) = p.get("control").filter(|v| !v.is_null()) {
                let name = name.as_str().ok_or("control must be fillStroke, coloring, drawMode or screenMode")?;
                let flag = app
                    .ui
                    .toolbar_show
                    .get_mut(name)
                    .ok_or_else(|| format!("unknown control `{name}`: fillStroke, coloring, drawMode or screenMode"))?;
                *flag = on.unwrap_or(!*flag);
            }
            Ok(app.ui.toolbar_show.json())
        })(),
        "toolbar.reset" => (|| {
            if let Some(name) = crate::custom_toolbars::target(&app.ui, p)? {
                // A toolbar made with New Toolbar goes back to blank, as it was made.
                return edit_custom(app, &name, |cats| {
                    cats.clear();
                    Ok(())
                });
            }
            *current(&mut app.ui.toolbar_edits, app.ui.toolbar_advanced) = None;
            Ok(slots_json(app))
        })(),
        _ => return None,
    })
}

// ---------- the drawer ----------

/// The heading a tool is listed under: its Basic toolbar group, else one for the tools only the
/// Advanced toolbar shows.
fn heading(id: &str) -> &'static str {
    if let Some((cat, _)) = crate::toolbar::BASIC.iter().find(|(_, slots)| slots.iter().any(|s| s.contains(&id))) {
        return cat;
    }
    if let Some(addon) = vectorcraft_tools::addons::heading(id) {
        return addon;
    }
    match id {
        _ if id.starts_with("symbol") => "Symbols",
        _ if id.ends_with("Graph") => "Graph",
        _ if id.starts_with("perspective") => "Perspective",
        "artboard" => "Artboards",
        "slice" | "sliceSelection" => "Slices",
        "objectsOnPath" => "Modify",
        _ => "More",
    }
}

/// The headings in order, each with its tools in the toolbar's order.
pub(crate) fn sections() -> &'static [(&'static str, Vec<&'static ToolInfo>)] {
    static SECTIONS: OnceLock<Vec<(&'static str, Vec<&'static ToolInfo>)>> = OnceLock::new();
    SECTIONS.get_or_init(|| {
        let mut out: Vec<(&'static str, Vec<&'static ToolInfo>)> = crate::toolbar::BASIC.iter().map(|(cat, _)| (*cat, vec![])).collect();
        for tool in vectorcraft_tools::catalog::all_tools() {
            let h = heading(tool.id);
            match out.iter_mut().find(|(c, _)| *c == h) {
                Some((_, tools)) => tools.push(tool),
                None => out.push((h, vec![tool])),
            }
        }
        out.retain(|(_, tools)| !tools.is_empty());
        out
    })
}

/// A heading of the drawer and the flyout groups listed under it.
type Heading = (&'static str, Vec<&'static [ToolInfo]>);

/// The headings in order, each with the flyout groups listed under it (a group under the heading
/// most of its tools have, the first tool's on a tie), each in its tools' order: what the drawer
/// frames together.
fn groups() -> &'static [Heading] {
    static GROUPS: OnceLock<Vec<Heading>> = OnceLock::new();
    GROUPS.get_or_init(|| {
        let mut out: Vec<Heading> = sections().iter().map(|(cat, _)| (*cat, vec![])).collect();
        for g in TOOL_GROUPS.iter().chain(vectorcraft_tools::addons::GROUPS) {
            let Some(first) = g.first() else { continue };
            let count = |h: &str| g.iter().filter(|t| heading(t.id) == h).count();
            let h = g.iter().map(|t| heading(t.id)).fold(heading(first.id), |best, h| if count(h) > count(best) { h } else { best });
            match out.iter_mut().find(|(c, _)| *c == h) {
                Some((_, gs)) => gs.push(g),
                None => out.push((h, vec![g])),
            }
        }
        out.retain(|(_, gs)| !gs.is_empty());
        out
    })
}

/// The params of the `toolbar.addTool` that puts `tool` at `at` in the toolbar the drawer edits
/// (`toolbar`: the one made with New Toolbar it names, none: the Tools panel's). The command takes
/// the tool out of where it was itself and counts slots before that, as the drawer's `DropAt` does,
/// so an add and a move are the same call. `None` for a place the toolbar doesn't have.
fn drop_params(app: &VectorcraftApp, tool: &str, at: DropAt, toolbar: Option<&str>) -> Option<Value> {
    let slots = target_slots(app);
    let mut p = json!({ "tool": tool, "toolbar": toolbar });
    match at {
        DropAt::NewSlot { before, .. } => {
            p["place"] = json!(Place::Before.name());
            // Before the slot count is the end: no slot.
            if before < slots.len() {
                p["slot"] = json!(before);
            }
        }
        DropAt::Slot { slot, .. } => {
            let k = slots.iter().position(|s| s.first().is_some_and(|first| crate::toolbar::slot_id(first) == slot))?;
            p["place"] = json!(Place::Group.name());
            p["slot"] = json!(k);
        }
        _ => return None,
    }
    Some(p)
}

/// Runs `id` and shows a refusal in the status line.
fn run(app: &mut VectorcraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.ui.status = e;
    }
}

/// Whether the All Tools drawer lists `tool`.
fn listed_tool(tool: &str) -> bool {
    groups().iter().any(|(_, gs)| gs.iter().any(|g| g.iter().any(|t| t.id == tool)))
}

/// The All Tools drawer beside the toolbar `rail_out` came from, while `tools_drawer` is on. `opens`
/// is the side of the rail the drawer opens on, the canvas side.
pub(crate) fn drawer(app: &mut VectorcraftApp, ui: &mut Ui, rail_out: &ToolRailOutput<String>, opens: Side) {
    if !app.ui.tools_drawer {
        return;
    }
    let Some(opener) = rail_out.edit_toolbar.as_ref() else { return };
    let target = crate::custom_toolbars::drawer_target(&app.ui).map(str::to_string);

    let listed: Vec<DrawerGroup<String>> = groups()
        .iter()
        .map(|(cat, gs)| {
            let runs = gs.iter().map(|g| g.iter().map(|t| crate::toolbar::rail_tool(t.id)).collect()).collect();
            DrawerGroup::slotted(crate::i18n::colon(tl!(cat)), runs)
        })
        .collect();
    let toolbar: Vec<ToolSlot<String>> = target_slots(app)
        .iter()
        .filter_map(|s| Some(ToolSlot::new(crate::toolbar::slot_id(s.first()?), s.iter().map(|id| crate::toolbar::rail_tool(id)).collect())))
        .collect();
    // Basic and Advanced show which layout the Tools panel has; Reset is the footer's Restore Defaults.
    let command = |id: &str, p: Value| PanelRow::Command(CommandRef::with_params(id, p));
    let rows = vec![
        command("window.toolbarAdvanced", json!({ "advanced": false })),
        command("window.toolbarAdvanced", json!({ "advanced": true })),
        PanelRow::Separator,
        command("window.toolbar.new", json!({})),
        command("window.toolbar.manage", json!({})),
    ];
    // A Learn link's tool that no shown toolbar holds: its row wears the mark, scrolled into view once.
    let located = locate::wanted(&app.ui, &Home::Drawer);
    let reveal = locate::take_reveal(&mut app.ui, &Home::Drawer, listed_tool);

    let mut drawer = ToolbarDrawer::new(rail_out.response.rect, &listed, &toolbar, rail_out.dropped).opened_by(opener).opens(opens).menu(rows);
    if let Some(tool) = &located {
        drawer = drawer.mark(tool);
    }
    if let Some(tool) = &reveal {
        drawer = drawer.reveal(tool);
    }
    // Show: the controls under the Tools panel's tools; a toolbar made with New Toolbar has none.
    let mut toggled = None;
    let shown = &app.ui.toolbar_show;
    let out = drawer.show_with(ui, |ui| {
        if target.is_some() {
            return;
        }
        ui.horizontal(|ui| {
            for (name, label, icon) in crate::state::ToolbarShow::CONTROLS {
                let on = shown.get(name);
                let button = ToggleButton::new(tl!(label), Bind::given(on)).icon(IconSpec::svg(IconId::new(icon))).icon_only();
                if button.show(ui).response().clicked() {
                    toggled = Some(name);
                }
            }
        });
    });

    for edit in out.edits {
        match edit {
            ToolbarEdit::Add { tool, at, .. } => {
                if let Some(p) = drop_params(app, &tool.id, at, target.as_deref()) {
                    run(app, "toolbar.addTool", p);
                }
            }
            ToolbarEdit::Move { tool, at, .. } => {
                if let Some(p) = drop_params(app, &tool, at, target.as_deref()) {
                    run(app, "toolbar.addTool", p);
                }
            }
            ToolbarEdit::Remove { tool, .. } => run(app, "toolbar.removeTool", json!({ "tool": tool, "toolbar": target })),
            ToolbarEdit::Reset => run(app, "toolbar.reset", json!({ "toolbar": target })),
            _ => {}
        }
    }
    if let Some(name) = toggled {
        run(app, "toolbar.show", json!({ "control": name }));
    }
    if let Some(id) = out.select.as_deref().and_then(tool_info).map(|t| t.id) {
        app.select_tool(id);
    }
    if out.closed || out.select.is_some() {
        app.ui.tools_drawer = false;
    }
}

/// The drawer menu's Basic or Advanced: the Tools panel takes that toolbar, and the drawer goes on
/// to edit the Tools panel's toolbar, also from a toolbar made with New Toolbar.
pub(crate) fn pick_layout(app: &mut VectorcraftApp, advanced: bool) -> Result<(), String> {
    if crate::custom_toolbars::drawer_target(&app.ui).is_some() {
        app.run("window.editToolbar", json!({ "open": true }))?;
    }
    app.ui.toolbar_advanced = advanced;
    Ok(())
}

#[cfg(test)]
mod tests;
