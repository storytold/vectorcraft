//! Floating panels: panels dragged out of the dock float inside the window as groups of tabs.
//!
//! A dock tab, an icon of the icon column or a popped-out panel's title dragged out of the dock
//! floats that panel; the strip right of the dock's tabs floats the whole tabbed group. A floating
//! group moves by its title bar (or the strip right of its tabs, or the tab of a lone panel), and a
//! tab dragged out of a group floats on its own. Dropped on the tabs of one of the dock's groups
//! (lit up), a group's panels join that group as tabs; dropped on the upper or lower half of it,
//! they make a new group above or below it in the dock. Dropped on the icon column, which lights
//! up, they become icons there, the tabbed group's panels too; dropped anywhere else on the dock,
//! they go back to where they live in it. Dropped on another floating group's title bar or tabs,
//! they stack with that group. The × of a group puts its panels back in the dock too. The Tools panel floats the same way by its
//! title bar ([`crate::toolbar`]) and docks again on the window's left edge.
//!
//! Dropped on the upper or lower half of another group (a line shows where), a group joins it in
//! a set right above or below it: the groups stack top to bottom, each showing one of its panels,
//! and move together by the top group's title bar, whose × docks them all; dropped on its tabs (lit
//! up), its panels join that group as tabs. A group dragged by its tab strip (a lone panel's
//! tab, or the strip right of the tabs) leaves its set and floats on its own. A double-click on a
//! tab collapses a group to its tabs, and another expands it. A dock column whose panels don't fit
//! the window scrolls.
//!
//! `window.panel.float` (`onto`, `below`, `above`, `collapsed`) / `window.panel.dock` do the same
//! for agents. The groups, their sets and the Tools panel's position are kept with the preferences
//! and in user workspaces, and a position saved on a bigger window is clamped into this one when
//! drawn.

use egui::{CornerRadius, Id, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use serde_json::{Value, json};

use crate::state::{DockTab, FloatingPanels, UiState, all_panels};
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, dock, icons, panels, widgets};

/// Height of a floating group's title bar (its grip and ×).
pub const TITLE: f32 = 14.0;
/// Height of a floating group's tab strip.
pub const STRIP: f32 = 26.0;
/// Height of the bar between two groups of a set.
pub const GAP: f32 = 3.0;
/// The least height of an open panel in a dock column: more panels than fit scroll.
const MIN_TALL: f32 = 160.0;
/// How far from a group's top or bottom edge a drop stacks it above or below the group.
const EDGE: f32 = 7.0;
/// Where `window.panel.float` floats a group given no position; each further group 24 points
/// lower right.
const FLOAT_AT: [f32; 2] = [420.0, 120.0];
/// Where `window.panel.float {panel: "tools"}` floats the Tools panel given no position.
const TOOLS_AT: [f32; 2] = [60.0, 120.0];
/// Points the pointer travels from its press before a drop counts (a click on a title bar that
/// overlaps the dock doesn't dock its group).
const TRAVEL: f32 = 4.0;

/// What a move carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Moving {
    /// The floating group holding this panel.
    Panel(&'static str),
    /// The floating Tools panel.
    Tools,
}

/// A move in progress: what moves, where the pointer holds it from its top-left corner, and where
/// the press began.
#[derive(Clone, Debug)]
struct Move {
    what: Moving,
    grab: Vec2,
    from: Pos2,
    original: UiState,
}

/// Where a moved group or Tools panel lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Drop {
    /// Back in the dock (the Tools panel: at the window's left edge).
    Dock,
    /// Stacked with the floating group holding this panel.
    Stack(&'static str),
    /// In a set with the floating group holding this panel, below it.
    Below(&'static str),
    /// In a set with the floating group holding this panel, above it (the top group of its set).
    Above(&'static str),
    /// In the dock as a new column at this place (1: next to the icon column, counting left).
    Column(u32),
    /// In the dock's group showing this panel: as tabs, or as a new group above or below it.
    Root(&'static str, RootZone),
    /// In the icon column, as icons (`window.panel.dock {icon: true}`).
    Icons,
}

/// Where a drop on one of the dock's groups puts the moved panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootZone {
    /// With the group, as tabs.
    Tab,
    /// In a new group right above it.
    Top,
    /// In a new group right below it.
    Bottom,
}

impl RootZone {
    /// The `zone` of `window.panel.dock` / `window.panel.move`.
    pub(crate) fn id(self) -> &'static str {
        match self {
            RootZone::Tab => "tab",
            RootZone::Top => "top",
            RootZone::Bottom => "bottom",
        }
    }
}

/// One of the dock's groups as last drawn (egui temp memory under [`root_groups_id`]): its panels,
/// the one it shows, its bounds and its tab strip. A group dropped on it joins it ([`Drop::Root`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RootGroup {
    pub panels: Vec<&'static str>,
    pub shown: &'static str,
    pub rect: Rect,
    pub strip: Rect,
}

/// The dock's groups as last drawn, top to bottom.
pub(crate) fn root_groups_id() -> Id {
    Id::new("floating-root-groups")
}

fn move_id() -> Id {
    Id::new("floating-move")
}

fn suppressed_id() -> Id {
    move_id().with("suppressed-until-primary-press")
}

fn gesture_allowed(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.focused && !input.key_pressed(egui::Key::Escape))
        && !ctx.data(|data| data.get_temp::<bool>(suppressed_id()).unwrap_or(false))
}

/// The dock (tabbed group and icon column) as last drawn: a group dropped there docks.
pub(crate) fn dock_rect_id() -> Id {
    Id::new("floating-dock-rect")
}

/// The strip along the window's left edge where the Tools panel docks, as last laid out.
/// The icon column's bounds (egui temp memory, one frame old), where panels without a tab in the
/// tabbed group go back to when docked.
pub(crate) fn icons_rect_id() -> Id {
    Id::new("floating-icons-rect")
}

pub(crate) fn tools_zone_id() -> Id {
    Id::new("floating-tools-zone")
}

/// The area of the floating group whose first panel is `first` (of the set it heads).
pub fn area_id(first: &str) -> Id {
    Id::new(("floating-panels", first))
}

/// The floating group whose first panel is `first`, as last drawn (egui temp memory): in a set,
/// from the bar above its tabs to its panel's bottom.
pub(crate) fn group_rect_id(first: &str) -> Id {
    Id::new(("floating-group-rect", first))
}

/// Panel `id` (`window.panel`) as a static string and its English label, if it is a panel (a
/// library panel's label is its id: [`label`] gives its library's name).
fn panel(id: &str) -> Option<(&'static str, &'static str)> {
    all_panels().find(|(p, _)| *p == id).or_else(|| crate::panels::library_panel::intern(id).map(|p| (p, p)))
}

/// Panel `id`'s tab label: its English label, or a library panel's library name.
fn label(app: &VectorcraftApp, id: &str) -> String {
    crate::panels::library_panel::tab_label(app, id).unwrap_or_else(|| panel(id).map_or(id, |(_, label)| label).to_string())
}

/// The floating group holding panel `id`.
pub fn group_of(ui: &UiState, id: &str) -> Option<usize> {
    ui.floating_panels.iter().position(|g| g.panels.iter().any(|p| p == id))
}

/// Reorder tabs in a floating group by index, keeping the active tab on the same panel.
pub(crate) fn reorder_group_tab_indices(g: &mut FloatingPanels, from: usize, to: usize) {
    if from != to && from < g.panels.len() && to < g.panels.len() {
        let active = g.panels.get(g.active).cloned();
        let moved = g.panels.remove(from);
        g.panels.insert(to, moved);
        g.active = active.and_then(|id| g.panels.iter().position(|p| *p == id)).unwrap_or(0);
    }
}

/// The floating groups drawn with group `gi`, top to bottom: the groups of its set, or it alone.
pub fn set_of(ui: &UiState, gi: usize) -> Vec<usize> {
    match ui.floating_panels.get(gi).and_then(|g| g.column) {
        Some(c) => ui.floating_panels.iter().enumerate().filter(|(_, g)| g.column == Some(c)).map(|(i, _)| i).collect(),
        None => vec![gi],
    }
}

/// Group `gi` heads its set (or floats alone): it draws the set's title bar.
fn leads(ui: &UiState, gi: usize) -> bool {
    set_of(ui, gi).first() == Some(&gi)
}

/// Move the set of group `gi` to `pos`.
fn set_pos(ui: &mut UiState, gi: usize, pos: [f32; 2]) {
    for i in set_of(ui, gi) {
        if let Some(g) = ui.floating_panels.get_mut(i) {
            g.pos = pos;
        }
    }
}

/// The panels of the groups `set`, in order.
fn ids_in(ui: &UiState, set: &[usize]) -> Vec<&'static str> {
    set.iter().filter_map(|&i| ui.floating_panels.get(i)).flat_map(ids_of).collect()
}

/// The panel shown by group `gi`.
fn shown(ui: &UiState, gi: usize) -> Option<&'static str> {
    let g = ui.floating_panels.get(gi)?;
    let ids = ids_of(g);
    ids.get(g.active).or(ids.first()).copied()
}

/// Put the groups `moved` in a set with the group holding `onto` (not one of them), right below or
/// right above it, at its position.
pub fn attach(ui: &mut UiState, moved: &[usize], onto: &str, below: bool) {
    let Some(target) = group_of(ui, onto).filter(|t| !moved.contains(t)) else { return };
    let column = match ui.floating_panels.get(target).and_then(|g| g.column) {
        Some(c) => c,
        None => ui.floating_panels.iter().filter_map(|g| g.column).max().map_or(1, |c| c.saturating_add(1)),
    };
    let Some((pos, docked)) = ui.floating_panels.get(target).map(|g| (g.pos, g.docked)) else { return };
    let (mut taken, mut kept) = (vec![], vec![]);
    for (i, mut g) in ui.floating_panels.drain(..).enumerate() {
        if i == target {
            g.column = Some(column);
        }
        if moved.contains(&i) {
            g.column = Some(column);
            g.pos = pos;
            g.docked = docked;
            taken.push(g);
        } else {
            kept.push(g);
        }
    }
    let at = kept.iter().position(|g| g.panels.iter().any(|p| p == onto)).map_or(kept.len(), |i| if below { i + 1 } else { i });
    kept.splice(at..at, taken);
    ui.floating_panels = kept;
    FloatingPanels::tidy(&mut ui.floating_panels);
    if let Some(tab) = shown_tab(ui) {
        ui.dock_tab = tab;
    }
}

/// The panels of group `g`, as static ids.
fn ids_of(g: &FloatingPanels) -> Vec<&'static str> {
    g.panels.iter().filter_map(|p| panel(p)).map(|(id, _)| id).collect()
}

/// Saved dock tab order, appending any tabs missing from older preferences.
pub(crate) fn resolved_dock_tab_order(ui: &UiState) -> Vec<DockTab> {
    if ui.dock_tab_order.is_empty() {
        DockTab::ALL.to_vec()
    } else {
        let mut order = ui.dock_tab_order.clone();
        for tab in DockTab::ALL {
            if !order.contains(&tab) {
                order.push(tab);
            }
        }
        order
    }
}

/// Persist dock tab order when it differs from the default visible strip.
pub(crate) fn set_dock_tab_order(ui: &mut UiState, visible: Vec<DockTab>) {
    let default = DockTab::ALL.iter().copied().filter(|t| group_of(ui, t.info().0).is_none()).collect::<Vec<_>>();
    ui.dock_tab_order = if visible == default { Vec::new() } else { visible };
}

/// The dock tabs left in the dock, in tab order.
pub fn docked_tabs(ui: &UiState) -> impl Iterator<Item = DockTab> + '_ {
    resolved_dock_tab_order(ui).into_iter().filter(|t| group_of(ui, t.info().0).is_none())
}

/// The dock tab the tabbed group shows: the chosen one, or the first left when it floats.
pub fn shown_tab(ui: &UiState) -> Option<DockTab> {
    docked_tabs(ui).find(|t| *t == ui.dock_tab).or_else(|| docked_tabs(ui).next())
}

/// Take `ids` out of the floating groups, dropping the groups left empty.
fn take(ui: &mut UiState, ids: &[&str]) {
    ui.floating_panels.retain_mut(|g| {
        let active = g.panels.get(g.active).filter(|p| !ids.contains(&p.as_str())).cloned();
        g.panels.retain(|p| !ids.contains(&p.as_str()));
        g.active = active.and_then(|a| g.panels.iter().position(|p| *p == a)).unwrap_or(0);
        !g.panels.is_empty()
    });
    FloatingPanels::tidy(&mut ui.floating_panels);
}

/// Float `ids` (out of the dock or of the group they were in) as a new group with its top-left
/// corner at `pos`, showing `active`.
pub fn float(ui: &mut UiState, ids: &[&str], active: &str, pos: Pos2) {
    take(ui, ids);
    if ui.open_panel.as_deref().is_some_and(|p| ids.contains(&p)) {
        ui.open_panel = None;
    }
    let panels: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
    let active = panels.iter().position(|p| p == active).unwrap_or(0);
    ui.floating_panels.push(FloatingPanels { panels, active, pos: [pos.x, pos.y], ..Default::default() });
    // The tabbed group shows another of its tabs when the one it showed floats.
    if let Some(tab) = shown_tab(ui) {
        ui.dock_tab = tab;
    }
}

/// Stack `ids` with the floating group holding `onto` (not one of them), showing `active`.
pub fn stack(ui: &mut UiState, ids: &[&str], active: &str, onto: &str) {
    take(ui, ids);
    if ui.open_panel.as_deref().is_some_and(|p| ids.contains(&p)) {
        ui.open_panel = None;
    }
    if let Some(g) = group_of(ui, onto).and_then(|i| ui.floating_panels.get_mut(i)) {
        g.panels.extend(ids.iter().map(|id| id.to_string()));
        g.active = g.panels.iter().position(|p| p == active).unwrap_or(g.active);
    }
    if let Some(tab) = shown_tab(ui) {
        ui.dock_tab = tab;
    }
}

/// Put `ids` back in the dock: the dock tabs in the tabbed group (showing `active`, or the first of
/// them), the other panels in the icon column.
pub fn dock(ui: &mut UiState, ids: &[&str], active: &str) {
    take(ui, ids);
    if let Some(tab) = DockTab::from_id(active).or_else(|| ids.iter().find_map(|id| DockTab::from_id(id))) {
        ui.dock_tab = tab;
    }
    ui.dock = true;
}

/// `pos`, a saved top-left corner, moved so that a `size` box there is inside `screen` (its
/// top-left corner first when it doesn't fit). Pulled in by whole points: egui rounds an area's
/// position, which would push a box of fractional size half a point past the edge.
pub fn clamp(pos: [f32; 2], size: Vec2, screen: Rect) -> Pos2 {
    let [x, y] = pos;
    let (x, y) = if x.is_finite() && y.is_finite() { (x, y) } else { (screen.left() + 60.0, screen.top() + 100.0) };
    let fit = |v: f32, max: f32| if v > max { max.floor() } else { v };
    pos2(fit(x, screen.right() - size.x).max(screen.left()), fit(y, screen.bottom() - size.y).max(screen.top()))
}

/// Start moving `what` with the pointer, held `grab` from its top-left corner.
fn start(app: &VectorcraftApp, ctx: &egui::Context, what: Moving, grab: Vec2) {
    if !gesture_allowed(ctx) {
        return;
    }
    let Some(from) = ctx.input(|i| i.pointer.press_origin().or(i.pointer.interact_pos())) else { return };
    ctx.data_mut(|d| d.insert_temp(move_id(), Move { what, grab, from, original: app.ui.clone() }));
}

/// Where the pointer holds a group torn off by a press `left` points right of where the group's
/// tab strip will begin (under its title bar, past its 1-point border).
pub(crate) fn strip_grab(ctx: &egui::Context, left: f32) -> Vec2 {
    let x = ctx.input(|i| i.pointer.press_origin()).map_or(12.0, |p| p.x - left);
    vec2(1.0 + x.max(0.0), 1.0 + TITLE + STRIP / 2.0)
}

/// Float `ids` under the pointer, held `grab` from the new group's top-left corner, and carry on
/// moving them with the pointer (a tab or group dragged out of the dock or out of its group).
pub(crate) fn tear(app: &mut VectorcraftApp, ctx: &egui::Context, ids: &[&'static str], active: &'static str, grab: Vec2) {
    // A restored popout may retain its drag response in the cancellation frame. Do not
    // immediately tear it out again after track restored the gesture's original layout.
    if !gesture_allowed(ctx) {
        return;
    }
    let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    let original = app.ui.clone();
    let pos = at - grab;
    if let Err(error) = app.run("window.panel.float", json!({"panel":active,"group":ids.len() > 1,"x":pos.x,"y":pos.y})) {
        app.ui.status = error;
        return;
    }
    start(app, ctx, Moving::Panel(active), grab);
    ctx.data_mut(|data| {
        if let Some(mut drag) = data.get_temp::<Move>(move_id()) {
            drag.original = original;
            data.insert_temp(move_id(), drag);
        }
    });
}

/// The Tools panel's title bar was pressed (`title`, the panel's bounds `bounds`): docked, dragging
/// it out of the panel floats the panel under the pointer; floating at `floating`, a drag moves it.
pub(crate) fn drag_tools(app: &mut VectorcraftApp, ctx: &egui::Context, title: &egui::Response, bounds: Rect, floating: Option<Pos2>) {
    if !gesture_allowed(ctx) {
        return;
    }
    let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    match floating {
        Some(pos) if title.drag_started() => start(app, ctx, Moving::Tools, at - pos),
        None if title.dragged() && !bounds.contains(at) => {
            // Held where the press grabbed the title bar.
            let grab = ctx.input(|i| i.pointer.press_origin()).map_or(vec2(10.0, 7.0), |p| p - bounds.min);
            let p = at - grab;
            start(app, ctx, Moving::Tools, grab);
            app.ui.toolbar_pos = Some([p.x, p.y]);
        }
        _ => {}
    }
}

/// Where the pointer at `at` would drop `what`, and the zone that lights up for it.
pub(crate) fn drop_target(app: &VectorcraftApp, ctx: &egui::Context, what: Moving, at: Pos2) -> Option<(Drop, Rect)> {
    let zone = |id: Id| ctx.data(|d| d.get_temp::<Rect>(id)).filter(|r| r.contains(at));
    let Moving::Panel(id) = what else { return zone(tools_zone_id()).map(|r| (Drop::Dock, r)) };
    let own = group_of(&app.ui, id);
    let moving = own.map(|i| set_of(&app.ui, i)).unwrap_or_default();
    // The left edge of the icon column or of a dock column docks the moved groups as a new column
    // there.
    if app.ui.dock && app.ui.screen_mode < 3 {
        let icons = ctx.data(|d| d.get_temp::<Rect>(icons_rect_id()));
        let columns = ctx.data(|d| d.get_temp::<Vec<Rect>>(columns_id())).unwrap_or_default();
        for (k, r) in icons.into_iter().chain(columns).enumerate() {
            if Rect::from_x_y_ranges((r.left() - EDGE - 3.0)..=(r.left() + EDGE - 1.0), r.y_range()).contains(at) {
                let line = Rect::from_x_y_ranges((r.left() - 2.0)..=(r.left() + 2.0), r.y_range());
                return Some((Drop::Column(u32::try_from(k + 1).unwrap_or(u32::MAX)), line));
            }
        }
    }
    // Another group (floating or docked): its tabs (and a set's title bar) stack the moved groups
    // with it as tabs; the upper half of the group (and just above a set) puts them in its set
    // right above it, the lower half (and just below it) right below it.
    let columns = ctx.data(|d| d.get_temp::<Vec<Rect>>(columns_id())).unwrap_or_default();
    for (i, g) in app.ui.floating_panels.iter().enumerate() {
        let Some((first, _)) = g.panels.first().and_then(|p| panel(p)).filter(|_| !moving.contains(&i)) else { continue };
        let Some(mut r) = ctx
            .data(|d| d.get_temp::<Rect>(group_rect_id(first)))
            .or_else(|| ctx.memory(|m| m.area_rect(Id::new("vectorcraft-panel-docking").with(("floating", &first.to_string())))))
        else {
            continue;
        };
        // A visible window owns hits inside its body. Old rectangles and lower windows
        // must not capture a drop through another panel or popup; edge insertion remains
        // available just outside the window. Dock columns reserve their own panel space.
        if g.docked.is_none() && r.contains(at) {
            let area = if app.ui.docking.is_some() && !crate::panel_docking::decorated(g) {
                Id::new("vectorcraft-panel-docking").with(("floating", &first.to_string()))
            } else {
                let lead = set_of(&app.ui, i).first().and_then(|&index| app.ui.floating_panels.get(index)).and_then(|group| group.panels.first());
                area_id(lead.map_or(first, String::as_str))
            };
            let moving_areas: Vec<Id> = moving
                .iter()
                .filter_map(|&index| app.ui.floating_panels.get(index).and_then(|group| group.panels.first()))
                .flat_map(|panel| [area_id(panel), Id::new("vectorcraft-panel-docking").with(("floating", panel))])
                .collect();
            let owner = ctx.memory(|memory| {
                let layers: Vec<_> = memory.layer_ids().collect();
                layers.into_iter().rev().find(|layer| {
                    !moving_areas.contains(&layer.id)
                        && memory.areas().is_visible(layer)
                        && memory.area_rect(layer.id).is_some_and(|rect| rect.contains(at))
                })
            });
            if owner.is_some_and(|layer| layer.id != area) {
                continue;
            }
        }
        // In a dock column that scrolls, only the part in view: a group scrolled away takes nothing.
        if let Some(col) = g.docked.and_then(|d| columns.get(usize::try_from(d).ok()?.checked_sub(1)?)) {
            r = r.intersect(*col);
            if r.height() <= 0.0 || r.width() <= 0.0 {
                continue;
            }
        }
        let line = |y: f32| Rect::from_x_y_ranges(r.x_range(), (y - 2.0)..=(y + 2.0));
        let lead = leads(&app.ui, i);
        let head = Rect::from_min_size(r.min, vec2(r.width(), if lead { 1.0 + TITLE + STRIP } else { GAP + STRIP }));
        if head.contains(at) {
            return Some((Drop::Stack(first), head));
        }
        let above = if lead { EDGE } else { 0.0 };
        if Rect::from_x_y_ranges(r.x_range(), (r.top() - above)..=(r.bottom() + EDGE)).contains(at) {
            let mid = (head.bottom() + r.bottom()) / 2.0;
            return Some(if at.y < mid { (Drop::Above(first), line(r.top())) } else { (Drop::Below(first), line(r.bottom())) });
        }
    }
    // A dock column's room under its groups puts them at its bottom.
    for ((_, set), col) in docked_sets(&app.ui).into_iter().zip(columns) {
        if !col.contains(at) || set.iter().any(|i| moving.contains(i)) {
            continue;
        }
        let Some(last) = set.last().and_then(|&i| app.ui.floating_panels.get(i)).and_then(|g| g.panels.first()).and_then(|p| panel(p)) else {
            continue;
        };
        let y = ctx.data(|d| d.get_temp::<Rect>(group_rect_id(last.0))).map_or(col.bottom(), |r| r.bottom()).min(col.bottom() - 2.0);
        return Some((Drop::Below(last.0), Rect::from_x_y_ranges(col.x_range(), (y - 2.0)..=(y + 2.0))));
    }
    let shown = app.ui.dock && app.ui.screen_mode < 3;
    // The icon column takes the panels as icons. (Mid-frame the dock's rect may not take it in yet.)
    if let Some(icons) = zone(icons_rect_id()).filter(|_| shown) {
        return Some((Drop::Icons, icons));
    }
    let dock = zone(dock_rect_id()).filter(|_| shown)?;
    // One of the dock's groups: its tabs take the panels as tabs, its upper or lower half puts them
    // in a new group above or below it. The group they came from (being dragged by a tab) takes
    // nothing: the shared dock handles a tab moved within it.
    let moved: Vec<&str> = std::iter::once(id).chain(ids_in(&app.ui, &moving)).collect();
    let groups = ctx.data(|d| d.get_temp::<Vec<RootGroup>>(root_groups_id())).unwrap_or_default();
    if let Some(group) = groups.iter().find(|g| g.rect.contains(at)) {
        let anchor = Some(group.shown).filter(|p| !moved.contains(p)).or_else(|| group.panels.iter().copied().find(|p| !moved.contains(p)));
        if let Some(anchor) = anchor {
            if group.strip.contains(at) {
                return Some((Drop::Root(anchor, RootZone::Tab), group.strip));
            }
            let mid = group.rect.center().y;
            return Some(if at.y < mid {
                (Drop::Root(anchor, RootZone::Top), Rect::from_x_y_ranges(group.rect.x_range(), group.rect.top()..=mid))
            } else {
                (Drop::Root(anchor, RootZone::Bottom), Rect::from_x_y_ranges(group.rect.x_range(), mid..=group.rect.bottom()))
            });
        }
    }
    // Panels that aren't tabs of the tabbed group (or all of them, with it collapsed) go back to
    // their icons: light up the icon column, where they land, rather than the tabs.
    let tabbed = !app.ui.dock_collapsed
        && own.and_then(|i| app.ui.floating_panels.get(i)).is_some_and(|g| ids_of(g).into_iter().any(|p| DockTab::from_id(p).is_some()));
    let lit = if tabbed { dock } else { ctx.data(|d| d.get_temp::<Rect>(icons_rect_id())).unwrap_or(dock) };
    Some((Drop::Dock, lit))
}

/// Light up a drop zone: over the docked panels, under the floating ones (the moved one too).
pub(crate) fn highlight(ctx: &egui::Context, zone: Rect) {
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, Id::new("floating-drop-zone")));
    painter.rect(zone.shrink(1.0), CornerRadius::same(2), t.accent.gamma_multiply(0.2), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
}

/// Light up the tabs a drop stacks the moved panels with, over the floating and docked groups.
fn tab_zone(ctx: &egui::Context, tabs: Rect) {
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("floating-drop-tabs")));
    painter.rect(tabs.shrink(1.0), CornerRadius::same(2), t.accent.gamma_multiply(0.25), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
}

/// Show where a drop puts the moved groups in a set: a line over the floating groups.
fn insertion_line(ctx: &egui::Context, line: Rect) {
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("floating-drop-line")));
    painter.rect_filled(line, CornerRadius::same(2), t.accent);
}

pub(crate) fn preview(ctx: &egui::Context, target: Drop, rect: Rect) {
    match target {
        Drop::Below(_) | Drop::Above(_) | Drop::Column(_) => insertion_line(ctx, rect),
        Drop::Stack(_) | Drop::Root(..) => tab_zone(ctx, rect),
        Drop::Dock | Drop::Icons => highlight(ctx, rect),
    }
}

/// Carry a move on (at the start of a frame, before the panels are drawn): the group or Tools panel
/// follows the pointer and the zone it would drop on lights up; the release drops it there.
pub fn track(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if ctx.input(|input| input.pointer.primary_pressed()) {
        ctx.data_mut(|data| data.remove::<bool>(suppressed_id()));
    }
    let before = app.ui.clone();
    let Some(m) = ctx.data(|d| d.get_temp::<Move>(move_id())) else { return };
    if m.original.docking_generation != app.ui.docking_generation {
        ctx.data_mut(|data| {
            data.remove::<Move>(move_id());
            data.insert_temp(suppressed_id(), true);
        });
        return;
    }
    if ctx.input(|input| !input.focused || input.key_pressed(egui::Key::Escape)) {
        app.ui.docking = m.original.docking;
        app.ui.docking_hidden = m.original.docking_hidden;
        app.ui.docking_icons = m.original.docking_icons;
        app.ui.docking_collapsed = m.original.docking_collapsed;
        app.ui.floating_panels = m.original.floating_panels;
        app.ui.open_panel = m.original.open_panel;
        app.ui.dock_tab = m.original.dock_tab;
        app.ui.toolbar_pos = m.original.toolbar_pos;
        ctx.data_mut(|data| {
            data.remove::<Move>(move_id());
            data.insert_temp(suppressed_id(), true);
        });
        return;
    }
    let at = ctx.input(|i| i.pointer.interact_pos());
    let target = at.filter(|at| at.distance(m.from) > TRAVEL).and_then(|at| drop_target(app, ctx, m.what, at));
    if ctx.input(|i| i.pointer.primary_down()) {
        if let Some(p) = at.map(|at| at - m.grab)
            && p.x.is_finite()
            && p.y.is_finite()
        {
            match m.what {
                Moving::Panel(id) => {
                    if let Some(gi) = group_of(&app.ui, id) {
                        set_pos(&mut app.ui, gi, [p.x, p.y]);
                    }
                }
                Moving::Tools if app.ui.toolbar_pos.is_some() => app.ui.toolbar_pos = Some([p.x, p.y]),
                Moving::Tools => {}
            }
        }
        if let Err(error) = crate::panel_docking::reconcile_native(app, before.clone()) {
            app.ui.status = error;
        }
        match target {
            Some((Drop::Below(_) | Drop::Above(_) | Drop::Column(_), line)) => insertion_line(ctx, line),
            Some((Drop::Stack(_) | Drop::Root(..), tabs)) => tab_zone(ctx, tabs),
            Some((_, zone)) => highlight(ctx, zone),
            None => {}
        }
        return;
    }
    ctx.data_mut(|d| d.remove::<Move>(move_id()));
    let Some((drop, _)) = target else { return };
    let Moving::Panel(id) = m.what else {
        app.ui.toolbar_pos = None;
        return;
    };
    // The moved set: its groups' panels, showing the top group's.
    let Some(gi) = group_of(&app.ui, id) else { return };
    let set = set_of(&app.ui, gi);
    let ids = ids_in(&app.ui, &set);
    let active = set.first().and_then(|&i| shown(&app.ui, i)).unwrap_or(id);
    // Icon panels dropped on the icon column go back to their icons, as on the rest of the dock.
    let drop = match drop {
        Drop::Icons if ids.iter().all(|p| crate::state::ICON_PANELS.iter().any(|entry| entry.0 == *p)) => Drop::Dock,
        drop => drop,
    };
    if matches!(drop, Drop::Root(..) | Drop::Icons) {
        // The shared layout puts them in the dock's tree, group by group: the first where it was
        // dropped, the others with it as tabs.
        let leads: Vec<&'static str> = set.iter().filter_map(|&i| shown(&app.ui, i)).collect();
        for (k, lead) in leads.iter().enumerate() {
            let params = match (drop, leads.first()) {
                (Drop::Root(anchor, zone), _) if k == 0 => json!({"panel": lead, "group": true, "anchor": anchor, "zone": zone.id()}),
                (Drop::Root(..), Some(first)) => json!({"panel": lead, "group": true, "anchor": first, "zone": "tab"}),
                _ => json!({"panel": lead, "group": true, "icon": true}),
            };
            if let Err(error) = app.run("window.panel.dock", params) {
                app.ui.status = error;
                break;
            }
        }
        return;
    }
    match drop {
        Drop::Root(..) | Drop::Icons => {}
        Drop::Dock => {
            dock(&mut app.ui, &ids, active);
            // One that went back to its icon pops out of it, so it doesn't seem to vanish.
            if app.ui.dock_collapsed || DockTab::from_id(active).is_none() {
                app.ui.open_panel = Some(active.to_string());
            }
        }
        Drop::Stack(onto) => stack(&mut app.ui, &ids, active, onto),
        Drop::Below(onto) => attach(&mut app.ui, &set, onto, true),
        Drop::Above(onto) => attach(&mut app.ui, &set, onto, false),
        Drop::Column(slot) => dock_column(&mut app.ui, &set, slot),
    }
    if let Err(error) = crate::panel_docking::reconcile_native(app, before) {
        app.ui.status = error;
    }
}

/// What a press on a floating or docked group did.
enum Action {
    /// Show the group's `k`th tab.
    Activate(usize, usize),
    /// A double-click on a tab or the strip right of the tabs collapses the group to its tabs, or
    /// expands it.
    Collapse(usize),
    /// Its × puts the panels of the set's groups back in the dock.
    Close(Vec<usize>),
    /// Move the set of the group holding this panel, drawn at this corner.
    Move(&'static str, Pos2),
    /// Take this group out of its set and move it on its own; its tab strip began this far right.
    Detach(usize, f32),
    /// Float this panel out of its group; its tab began this far right.
    Tear(&'static str, f32),
    /// Reorder tabs in floating group `gi` from index `from` to `to`.
    Reorder(usize, usize, usize),
    /// Float the docked column of this group, its top-left corner this far from the pointer.
    Undock(usize, Vec2),
}

/// A floating group as drawn: its index, panels, the one shown and whether it's collapsed.
struct Shown {
    gi: usize,
    ids: Vec<&'static str>,
    active: &'static str,
    collapsed: bool,
}

/// The groups `set` as drawn, top to bottom.
fn shown_groups(ui: &UiState, set: &[usize]) -> Vec<Shown> {
    set.iter()
        .filter_map(|&gi| {
            let g = ui.floating_panels.get(gi)?;
            let ids = ids_of(g);
            let active = ids.get(g.active).or(ids.first()).copied()?;
            Some(Shown { gi, ids, active, collapsed: g.collapsed })
        })
        .collect()
}

/// The groups `set` heads are docked as a dock column.
fn docked(ui: &UiState, set: &[usize]) -> bool {
    set.first().and_then(|&i| ui.floating_panels.get(i)).is_some_and(|g| g.docked.is_some())
}

/// The floating groups, each a stack of tabs, stacked in their sets under a title bar. Hidden with
/// the dock (Tab, Presentation Mode). Sets docked as dock columns are drawn by [`docked_columns`].
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.ui.floating_panels.is_empty() || !app.ui.dock || app.ui.screen_mode >= 3 {
        crate::panel_docking::finish_native_drop(app, ctx);
        return;
    }
    let mut action = None;
    for gi in 0..app.ui.floating_panels.len() {
        if leads(&app.ui, gi) && (app.ui.docking.is_none() || app.ui.floating_panels.get(gi).is_some_and(crate::panel_docking::decorated)) {
            let set = set_of(&app.ui, gi);
            if !docked(&app.ui, &set) {
                show_set(app, ctx, &set, &mut action);
            }
        }
    }
    apply(app, ctx, action);
    crate::panel_docking::finish_native_drop(app, ctx);
}

/// Carry out what a press on a floating or docked group did.
fn apply(app: &mut VectorcraftApp, ctx: &egui::Context, action: Option<Action>) {
    if action.is_none() {
        return;
    }
    if matches!(action, Some(Action::Move(..) | Action::Detach(..) | Action::Tear(..) | Action::Undock(..))) && !gesture_allowed(ctx) {
        return;
    }
    let before = app.ui.clone();
    let skip_reconcile = matches!(action, Some(Action::Reorder(_, _, _)));
    match action {
        Some(Action::Activate(gi, k)) => {
            if let Some(g) = app.ui.floating_panels.get_mut(gi)
                && g.active != k
            {
                g.active = k;
                g.collapsed = false;
            }
        }
        Some(Action::Collapse(gi)) => {
            if let Some(g) = app.ui.floating_panels.get_mut(gi) {
                g.collapsed = !g.collapsed;
            }
        }
        Some(Action::Close(set)) => {
            let ids = ids_in(&app.ui, &set);
            let active = set.first().and_then(|&i| shown(&app.ui, i)).unwrap_or_default();
            dock(&mut app.ui, &ids, active);
        }
        Some(Action::Move(id, pos)) => {
            // From where it is drawn (a position saved on a bigger window was clamped into this one).
            if let Some(gi) = group_of(&app.ui, id) {
                set_pos(&mut app.ui, gi, [pos.x, pos.y]);
            }
            if let Some(at) = ctx.input(|i| i.pointer.interact_pos()) {
                start(app, ctx, Moving::Panel(id), at - pos);
            }
        }
        Some(Action::Detach(gi, left)) => detach(app, ctx, gi, left),
        Some(Action::Tear(id, left)) => tear(app, ctx, &[id], id, strip_grab(ctx, left)),
        Some(Action::Reorder(gi, from, to)) => {
            let ids = app.ui.floating_panels.get(gi).and_then(|g| {
                let panel = g.panels.get(from)?;
                let before = g.panels.get(to)?;
                Some((panel.clone(), before.clone()))
            });
            if let Some((panel, before)) = ids
                && let Err(error) = crate::panel_docking::strip_tab_reorder(app, &panel, &before)
            {
                app.ui.status = error;
            }
            return;
        }
        Some(Action::Undock(gi, grab)) => {
            let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else { return };
            let Some(id) = set_of(&app.ui, gi).first().and_then(|&i| shown(&app.ui, i)) else { return };
            let p = at - grab;
            for i in set_of(&app.ui, gi) {
                if let Some(g) = app.ui.floating_panels.get_mut(i) {
                    g.docked = None;
                    g.pos = [p.x, p.y];
                }
            }
            FloatingPanels::tidy(&mut app.ui.floating_panels);
            start(app, ctx, Moving::Panel(id), grab);
        }
        None => {}
    }
    if !skip_reconcile && let Err(error) = crate::panel_docking::reconcile_native(app, before.clone()) {
        app.ui.status = error;
    }
    ctx.data_mut(|data| {
        if let Some(mut drag) = data.get_temp::<Move>(move_id()) {
            drag.original = before;
            data.insert_temp(move_id(), drag);
        }
    });
}

/// Take group `gi` out of its set (or its dock column) under the pointer, its tab strip having
/// begun `left` points from the window's left, and carry on moving it with the pointer.
fn detach(app: &mut VectorcraftApp, ctx: &egui::Context, gi: usize, left: f32) {
    let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    let Some(id) = shown(&app.ui, gi) else { return };
    let grab = strip_grab(ctx, left);
    let p = at - grab;
    if let Some(g) = app.ui.floating_panels.get_mut(gi) {
        g.column = None;
        g.docked = None;
        g.pos = [p.x, p.y];
    }
    FloatingPanels::tidy(&mut app.ui.floating_panels);
    start(app, ctx, Moving::Panel(id), grab);
}

/// Put the groups `set` in the dock as a new column at `slot` (1: next to the icon column, counting
/// left), moving the columns from there one further left.
pub fn dock_column(ui: &mut UiState, set: &[usize], slot: u32) {
    let slot = slot.max(1);
    for (i, g) in ui.floating_panels.iter_mut().enumerate() {
        if set.contains(&i) {
            g.docked = Some(slot);
        } else if let Some(d) = g.docked.filter(|d| *d >= slot) {
            g.docked = Some(d.saturating_add(1));
        }
    }
    FloatingPanels::tidy(&mut ui.floating_panels);
    if let Some(tab) = shown_tab(ui) {
        ui.dock_tab = tab;
    }
}

/// The dock columns, as last drawn (egui temp memory), from the one next to the icon column
/// leftwards.
pub(crate) fn columns_id() -> Id {
    Id::new("dock-columns")
}

/// The sets docked as columns, by their column number (next to the icon column first).
fn docked_sets(ui: &UiState) -> Vec<(u32, Vec<usize>)> {
    let mut sets: Vec<(u32, Vec<usize>)> = (0..ui.floating_panels.len())
        .filter(|&gi| leads(ui, gi))
        .filter_map(|gi| Some((ui.floating_panels.get(gi)?.docked?, set_of(ui, gi))))
        .collect();
    sets.sort_by_key(|(d, _)| *d);
    sets
}

/// The sets docked as columns, each a column left of the icon column (`ui`, the window's root, as
/// [`dock::show`] lays it out): a strip along its top that floats the column when dragged and
/// whose × puts its panels back in the dock, then its groups top to bottom. Returns the columns'
/// bounds, next to the icon column first.
pub(crate) fn docked_columns(app: &mut VectorcraftApp, ui: &mut egui::Ui) -> Vec<Rect> {
    let t = Tokens::get(ui.ctx());
    let mut rects = vec![];
    let mut action = None;
    for (_, set) in docked_sets(&app.ui) {
        let groups = shown_groups(&app.ui, &set);
        let Some(first) = groups.first().and_then(|g| g.ids.first().copied()) else { continue };
        let natural = groups.iter().flat_map(|g| g.ids.iter()).map(|id| dock::panel_width(id)).fold(200.0, f32::max);
        let panel = egui::Panel::right(Id::new(("dock-column", first)))
            .resizable(true)
            .default_size(natural)
            .size_range(200.0..=520.0)
            .frame(egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.5, t.border)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let width = ui.available_width();
                let (bar, _) = ui.allocate_exact_size(vec2(width, TITLE), Sense::hover());
                ui.painter().rect_filled(bar, 0.0, t.tab_strip);
                let close = Rect::from_center_size(bar.right_center() - vec2(10.0, 0.0), vec2(TITLE, TITLE));
                let grip = ui.interact(Rect::from_min_max(bar.min, pos2(close.left(), bar.bottom())), ui.id().with("column-bar"), Sense::drag());
                for k in 0..2 {
                    let y = bar.center().y - 1.5 + k as f32 * 3.0;
                    ui.painter().line_segment([pos2(bar.center().x - 9.0, y), pos2(bar.center().x + 9.0, y)], Stroke::new(0.6, t.text_disabled));
                }
                if grip.drag_started()
                    && let (Some(at), Some(&gi)) = (ui.input(|i| i.pointer.press_origin()), set.first())
                {
                    action = Some(Action::Undock(gi, at - bar.min));
                }
                grip.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tl!("Drag to float the panel group"));
                let x = ui.interact(close, ui.id().with("column-close"), Sense::click());
                icons::paint(ui, "x", close.shrink(3.0), if x.hovered() { t.text_strong } else { t.text_dim });
                if x.on_hover_text(tl!("Put back in the dock")).clicked() {
                    action = Some(Action::Close(set.clone()));
                }
                let n = groups.len() as f32;
                let open = groups.iter().filter(|g| !g.collapsed).count().max(1) as f32;
                // The open panels share the column's height, each at least `MIN_TALL`: more than fit
                // scroll, with the column.
                let tall = ((ui.available_height() - n * STRIP - (n - 1.0) * GAP - 24.0 * open) / open).max(MIN_TALL);
                widgets::panel_scroll(ui, egui::ScrollArea::vertical().id_salt(("dock-column-scroll", first)).auto_shrink([false, false]), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let width = ui.available_width();
                    let top = ui.cursor().top();
                    draw_groups(app, ui, &groups, width, tall, top, None, &mut action);
                });
            });
        rects.push(panel.response.rect);
    }
    ui.ctx().data_mut(|d| {
        let icons = d.get_temp::<Rect>(icons_rect_id());
        if let Some(left) = icons.map(|r| rects.iter().map(|r| r.left()).fold(r.left(), f32::min)) {
            d.insert_temp(dock::dock_left_id(), left);
        }
        d.insert_temp(columns_id(), rects.clone());
    });
    let ctx = ui.ctx().clone();
    apply(app, &ctx, action);
    rects
}

/// Draw the set of floating groups `set` (top to bottom) as one floating box.
fn show_set(app: &mut VectorcraftApp, ctx: &egui::Context, set: &[usize], action: &mut Option<Action>) {
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let groups = shown_groups(&app.ui, set);
    let Some((lead, first)) = groups.first().and_then(|g| Some((g, *g.ids.first()?))) else { return };
    let Some(saved) = app.ui.floating_panels.get(lead.gi).map(|g| g.pos) else { return };
    let given = set.iter().filter_map(|&i| app.ui.floating_panels.get(i)?.width).reduce(f32::max).map(|w| w.min(screen.width() - 16.0));
    let area = area_id(first);
    let size = ctx.memory(|m| m.area_rect(area)).map_or(vec2(dock::panel_width(lead.active), 200.0), |r| r.size());
    let pos = clamp(saved, size, screen);
    // The tabbed group's panels fill (Layers) or scroll (Properties) to near the window's bottom,
    // leaving room for their margins (a group that only just fitted would creep up a little each
    // frame); the open panels of a set share that height.
    let n = groups.len() as f32;
    let open = groups.iter().filter(|g| !g.collapsed).count().max(1) as f32;
    let tall = ((screen.bottom() - pos.y - TITLE - n * STRIP - (n - 1.0) * GAP - 60.0) / open).clamp(120.0, 560.0);
    let lead_active = lead.active;
    egui::Area::new(area).order(egui::Order::Middle).fixed_pos(pos).show(ctx, |ui| {
        let frame = egui::Frame::popup(ui.style()).fill(t.panel).corner_radius(CornerRadius::same(4)).inner_margin(egui::Margin::ZERO);
        frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let width = groups
                .iter()
                .map(|g| {
                    let tabs_width: f32 = g
                        .ids
                        .iter()
                        .map(|id| ui.painter().layout_no_wrap(tl!(&label(app, id)).to_string(), theme::semibold(12.0), t.text).size().x + 24.0)
                        .sum();
                    g.ids.iter().map(|id| dock::panel_width(id)).fold(tabs_width + 32.0, f32::max)
                })
                .fold(given.unwrap_or(0.0), f32::max);
            ui.set_width(width);
            // Title bar: a grip that moves the set, and the × that docks it.
            let (bar, _) = ui.allocate_exact_size(vec2(width, TITLE), Sense::hover());
            ui.painter().rect_filled(bar, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, t.tab_strip);
            let close = Rect::from_center_size(bar.right_center() - vec2(10.0, 0.0), vec2(TITLE, TITLE));
            let grip = ui.interact(Rect::from_min_max(bar.min, pos2(close.left(), bar.bottom())), ui.id().with("title"), Sense::drag());
            for k in 0..2 {
                let y = bar.center().y - 1.5 + k as f32 * 3.0;
                ui.painter().line_segment([pos2(bar.center().x - 9.0, y), pos2(bar.center().x + 9.0, y)], Stroke::new(0.6, t.text_disabled));
            }
            if grip.drag_started() {
                *action = Some(Action::Move(lead_active, pos));
            }
            grip.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tl!("Drag to move"));
            let x = ui.interact(close, ui.id().with("close"), Sense::click());
            icons::paint(ui, "x", close.shrink(3.0), if x.hovered() { t.text_strong } else { t.text_dim });
            if x.on_hover_text(tl!("Put back in the dock")).clicked() {
                *action = Some(Action::Close(set.to_vec()));
            }
            let alone = (groups.len() == 1).then_some(pos);
            draw_groups(app, ui, &groups, width, tall, bar.top(), alone, action);
        });
    });
}

/// Draw `groups` top to bottom, `width` wide, their open panels `tall`, the first one's bar having
/// begun at `top`: a bar between groups, each group's tab strip, then its shown panel. `alone`, a
/// lone floating group drawn at that corner: its tab strip moves it (else it takes its group out of
/// the set or dock column).
#[allow(clippy::too_many_arguments)]
fn draw_groups(
    app: &mut VectorcraftApp,
    ui: &mut egui::Ui,
    groups: &[Shown],
    width: f32,
    tall: f32,
    top: f32,
    alone: Option<Pos2>,
    action: &mut Option<Action>,
) {
    let t = Tokens::get(ui.ctx());
    let pointer = ui.input(|i| i.pointer.interact_pos());
    for (k, g) in groups.iter().enumerate() {
        let top = if k == 0 {
            top
        } else {
            let (gap, _) = ui.allocate_exact_size(vec2(width, GAP), Sense::hover());
            ui.painter().rect_filled(gap, 0.0, t.tab_strip);
            gap.top()
        };
        // Tab strip: a click shows a tab, a double-click on a tab or on the strip right of the tabs
        // collapses the group, a drag out of the strip floats a tab on its own; a lone tab or the
        // strip right of the tabs moves the group (out of its set).
        let (strip, _) = ui.allocate_exact_size(vec2(width, STRIP), Sense::hover());
        ui.painter().rect_filled(strip, 0.0, t.panel_darker);
        let lone = g.ids.len() == 1;
        let menu = Rect::from_center_size(strip.right_center() - vec2(14.0, 0.0), vec2(16.0, 16.0));
        let tab_area = Rect::from_min_max(strip.min, pos2((menu.left() - 4.0).max(strip.left()), strip.bottom()));
        let labels: Vec<String> = g.ids.iter().map(|id| label(app, id)).collect();
        let names: Vec<_> = g.ids.iter().zip(&labels).map(|(&id, label)| (id, label.as_str())).collect();
        let selected = g.ids.iter().position(|&id| id == g.active).unwrap_or(0);
        let out = dock::panel_tabs(ui, &format!("native-set-tabs-{}", g.gi), tab_area, &names, selected, theme::semibold(12.0), false);
        for (j, resp) in &out.responses {
            let Some(&id) = g.ids.get(*j) else { continue };
            if let Some((from, to)) = dock::tab_reorder_on_release(strip, pointer, resp, &out.responses) {
                *action = Some(Action::Reorder(g.gi, from, to));
            } else if resp.double_clicked() {
                *action = Some(Action::Collapse(g.gi));
            } else if resp.clicked() {
                *action = Some(Action::Activate(g.gi, *j));
            } else if lone && resp.drag_started() {
                *action = Some(match alone {
                    Some(pos) => Action::Move(id, pos),
                    None => Action::Detach(g.gi, strip.left()),
                });
            } else if !lone && resp.dragged() && pointer.is_some_and(|p| !strip.expand(4.0).contains(p)) {
                *action = Some(Action::Tear(id, resp.rect.left()));
            }
        }
        if let Some(j) = out.picked {
            *action = Some(Action::Activate(g.gi, j));
        }
        let rest = Rect::from_min_max(pos2(out.right, strip.top()), pos2(menu.left() - 4.0, strip.bottom()));
        if rest.width() > 0.0 {
            // Click as well as drag, so a press there no longer starts the move at once and a
            // double-click can collapse the group.
            let resp = ui.interact(rest, ui.id().with(("strip", g.gi)), Sense::click_and_drag());
            if resp.double_clicked() {
                *action = Some(Action::Collapse(g.gi));
            } else if resp.drag_started() {
                *action = Some(match alone {
                    Some(pos) => Action::Move(g.active, pos),
                    None => Action::Detach(g.gi, strip.left()),
                });
            }
            let tip = if alone.is_some() { tl!("Drag to move") } else { tl!("Drag to move out of the set") };
            resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tip);
        }
        panels::panel_menu(app, ui, g.active, menu);
        if !g.collapsed {
            dock::panel_body(app, ui, g.active, width, tall);
        }
        // Where the next widget would go: in a dock column, the ui's own bounds reach its bottom.
        let bottom = ui.cursor().top().max(strip.bottom());
        if let Some(&id) = g.ids.first() {
            let rect = Rect::from_min_max(pos2(strip.left(), top), pos2(strip.left() + width, bottom));
            ui.ctx().data_mut(|d| d.insert_temp(group_rect_id(id), rect));
        }
    }
}

/// `window.panel.float` and `window.panel.dock`; `None` for other commands.
pub fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let float = match id {
        "window.panel.float" => true,
        "window.panel.dock" => false,
        _ => return None,
    };
    Some(float_or_dock(app, float, p))
}

/// The `x`, `y` params: a top-left corner in screen points, if given.
fn position(p: &Value) -> Result<Option<Pos2>, String> {
    let coord = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Some(None),
        Some(v) => v.as_f64().filter(|f| f.abs() < 1e6).map(|f| Some(f as f32)),
    };
    match (coord("x"), coord("y")) {
        (Some(Some(x)), Some(Some(y))) => Ok(Some(pos2(x, y))),
        (Some(None), Some(None)) => Ok(None),
        _ => Err("x and y must both be numbers".into()),
    }
}

fn float_or_dock(app: &mut VectorcraftApp, float_it: bool, p: &Value) -> Result<Value, String> {
    let raw = p.get("panel").and_then(Value::as_str).unwrap_or_default().trim();
    let group = crate::menus::opt_bool(p, "group")?.unwrap_or(false);
    let at = position(p)?;
    if raw.eq_ignore_ascii_case("tools") || raw.eq_ignore_ascii_case("toolbar") {
        app.ui.toolbar_pos = float_it.then(|| at.map_or(TOOLS_AT, |p| [p.x, p.y]));
        app.ui.toolbar |= float_it;
        return Ok(json!({ "panel": "tools", "floating": float_it, "pos": app.ui.toolbar_pos }));
    }
    let Some(id) = crate::menus::normalize_panel(raw) else { return Err(format!("unknown panel `{raw}`")) };
    let own = group_of(&app.ui, id);
    // The panel, or with `group` its whole group: its floating group, or the tabs left in the dock.
    let ids: Vec<&'static str> = match (group, own.and_then(|i| app.ui.floating_panels.get(i))) {
        (true, Some(g)) => ids_of(g),
        (true, None) if DockTab::from_id(id).is_some() => docked_tabs(&app.ui).map(|t| t.info().0).collect(),
        _ => vec![id],
    };
    let collapsed = crate::menus::opt_bool(p, "collapsed")?;
    let onto = p.get("onto").and_then(Value::as_str);
    let below = p.get("below").and_then(Value::as_str);
    let above = p.get("above").and_then(Value::as_str);
    if [onto, below, above].iter().filter(|o| o.is_some()).count() > 1 {
        return Err("give only one of onto, below and above".into());
    }
    let slot = column_slot(&app.ui, p)?;
    app.ui.dock = true;
    // Its own group already (not a panel taken out of its group).
    let whole = own.filter(|&i| app.ui.floating_panels.get(i).is_some_and(|g| g.panels.len() == ids.len()));
    if !float_it {
        let Some(slot) = slot else {
            dock(&mut app.ui, &ids, id);
            return Ok(json!({ "panel": id, "floating": false }));
        };
        // A dock column of its own group (floated out of its group first) and the rest of its set.
        if whole.is_none() {
            float(&mut app.ui, &ids, id, Pos2::ZERO);
        }
        if let Some(gi) = group_of(&app.ui, id) {
            let set = set_of(&app.ui, gi);
            dock_column(&mut app.ui, &set, slot);
        }
        let column = group_of(&app.ui, id).and_then(|i| app.ui.floating_panels.get(i)).and_then(|g| g.docked);
        return Ok(json!({ "panel": id, "floating": false, "column": column }));
    }
    // Floating a docked group floats its dock column.
    if let Some(gi) = whole.filter(|&i| app.ui.floating_panels.get(i).is_some_and(|g| g.docked.is_some())) {
        for i in set_of(&app.ui, gi) {
            if let Some(g) = app.ui.floating_panels.get_mut(i) {
                g.docked = None;
            }
        }
        FloatingPanels::tidy(&mut app.ui.floating_panels);
    }
    if let Some((next, under)) = below.map(|b| (b, true)).or(above.map(|a| (a, false))) {
        let next = crate::menus::normalize_panel(next)
            .filter(|o| !ids.contains(o) && group_of(&app.ui, o).is_some())
            .ok_or("below and above must name a panel floating in another group")?;
        // The panels as a group of their own (their group already, or floated out), in the set.
        if whole.is_none() {
            float(&mut app.ui, &ids, id, Pos2::ZERO);
        }
        if let Some(gi) = group_of(&app.ui, id) {
            attach(&mut app.ui, &[gi], next, under);
        }
    } else {
        float_onto(app, &ids, id, own, onto, at)?;
    }
    if let Some(g) = group_of(&app.ui, id).and_then(|i| app.ui.floating_panels.get_mut(i)) {
        g.active = g.panels.iter().position(|q| q == id).unwrap_or(g.active);
        if let Some(c) = collapsed {
            g.collapsed = c;
        }
    }
    let gi = group_of(&app.ui, id);
    let g = gi.and_then(|i| app.ui.floating_panels.get(i));
    let set: Vec<&Vec<String>> =
        gi.map(|i| set_of(&app.ui, i)).unwrap_or_default().iter().filter_map(|&i| app.ui.floating_panels.get(i)).map(|g| &g.panels).collect();
    let (group, pos, collapsed, column) = (g.map(|g| &g.panels), g.map(|g| g.pos), g.is_some_and(|g| g.collapsed), g.and_then(|g| g.docked));
    Ok(json!({ "panel": id, "floating": column.is_none(), "group": group, "set": set, "pos": pos, "collapsed": collapsed, "column": column }))
}

/// The `column` param of `window.panel.dock`: `true` for a new dock column at the dock's left, or
/// the place of the new column (1: next to the icon column, counting left); none to dock the panel
/// where it lives.
fn column_slot(ui: &UiState, p: &Value) -> Result<Option<u32>, String> {
    let next = ui.floating_panels.iter().filter_map(|g| g.docked).max().unwrap_or(0).saturating_add(1);
    match p.get("column") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(next)),
        Some(v) => match v.as_u64().filter(|n| *n >= 1) {
            Some(n) => Ok(Some(u32::try_from(n).unwrap_or(u32::MAX).min(next))),
            None => Err("column must be true or a number from 1".into()),
        },
    }
}

/// Float `ids` (`id` among them; `own`, the group holding `id`) as their own group at `at`, or
/// stacked as tabs `onto` another group.
fn float_onto(
    app: &mut VectorcraftApp,
    ids: &[&'static str],
    id: &'static str,
    own: Option<usize>,
    onto: Option<&str>,
    at: Option<Pos2>,
) -> Result<(), String> {
    match onto {
        Some(onto) => {
            let onto = crate::menus::normalize_panel(onto)
                .filter(|o| !ids.contains(o) && group_of(&app.ui, o).is_some_and(|g| Some(g) != own))
                .ok_or("onto must name a panel floating in another group")?;
            stack(&mut app.ui, ids, id, onto);
        }
        // Its own group already: moved there (or left where it is), showing the panel.
        None => match own.and_then(|i| app.ui.floating_panels.get_mut(i)).filter(|g| g.panels.len() == ids.len()) {
            // Moved to `at` takes it out of its set; else it stays where it is.
            Some(g) => {
                if let Some(at) = at {
                    g.pos = [at.x, at.y];
                    g.column = None;
                    g.docked = None;
                    FloatingPanels::tidy(&mut app.ui.floating_panels);
                }
            }
            None => {
                let n = app.ui.floating_panels.len() as f32;
                let at = at.unwrap_or(pos2(FLOAT_AT[0] + 24.0 * n, FLOAT_AT[1] + 24.0 * n));
                float(&mut app.ui, ids, id, at);
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton};
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    const SCREEN: Vec2 = vec2(1400.0, 900.0);

    /// Frames of the whole window, with pointer input one event a frame (as the control
    /// channel's `ui.drag` sends it).
    struct Harness {
        app: VectorcraftApp,
        ctx: egui::Context,
        time: f64,
        screen: Vec2,
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    impl Harness {
        fn new() -> Self {
            Self::on(SCREEN)
        }

        /// The same window at another size. The icon column scrolls when it is too short for
        /// every icon, which hides the ones past the bottom — a test that counts them needs
        /// a window tall enough to show them all.
        fn on(screen: Vec2) -> Self {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
            let mut h = Self { app, ctx: egui::Context::default(), time: 0.0, screen };
            h.settle();
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.05;
            let screen = Rect::from_min_size(Pos2::ZERO, self.screen);
            let focused =
                events.iter().rev().find_map(|event| if let Event::WindowFocused(focused) = event { Some(*focused) } else { None }).unwrap_or(true);
            let raw = egui::RawInput { time: Some(self.time), screen_rect: Some(screen), focused, events, ..Default::default() };
            let app = &mut self.app;
            self.ctx
                .run_ui(raw, |ui| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                })
                .textures_delta
                .clear();
        }

        fn settle(&mut self) {
            for _ in 0..4 {
                self.frame(vec![]);
            }
        }

        /// Press at `from` and move to `to` in steps, the button still down.
        fn hold(&mut self, from: Pos2, to: Pos2) {
            self.frame(vec![Event::PointerMoved(from)]);
            self.frame(vec![button(from, true)]);
            for k in 1..=8 {
                self.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 8.0))]);
            }
        }

        fn release(&mut self, at: Pos2) {
            self.frame(vec![button(at, false)]);
            self.settle();
        }

        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.hold(from, to);
            self.release(to);
        }

        fn click(&mut self, at: Pos2) {
            self.frame(vec![Event::PointerMoved(at), button(at, true)]);
            self.release(at);
        }

        fn temp_rect(&self, id: Id) -> Rect {
            self.ctx.data(|d| d.get_temp::<Rect>(id)).unwrap()
        }

        /// The floating group whose first panel is `first`, as drawn.
        fn group(&self, first: &str) -> Rect {
            let native = group_of(&self.app.ui, first).and_then(|i| self.app.ui.floating_panels.get(i)).is_some_and(crate::panel_docking::decorated);
            let id = if self.app.ui.docking.is_some() && !native {
                Id::new("vectorcraft-panel-docking").with(("floating", &first.to_string()))
            } else {
                area_id(first)
            };
            self.ctx.memory(|m| m.area_rect(id)).unwrap()
        }

        /// A point on a floating group's title bar (left of its ×).
        fn title(&self, first: &str) -> Pos2 {
            let native = group_of(&self.app.ui, first).and_then(|i| self.app.ui.floating_panels.get(i)).is_some_and(crate::panel_docking::decorated);
            if self.app.ui.docking.is_some() && !native {
                let rect = self.group(first);
                return rect.left_top() + vec2(6.0, 16.0);
            }
            let r = self.group(first);
            pos2(r.left() + 30.0, r.top() + 1.0 + TITLE / 2.0)
        }

        /// The rects of the widgets that sense a drag and pass `keep`, top to bottom, left to right.
        fn widgets(&self, keep: impl Fn(Rect) -> bool) -> Vec<Rect> {
            let mut r: Vec<Rect> = self.ctx.viewport(|vp| {
                vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.sense.senses_drag() && keep(w.rect)).map(|w| w.rect).collect()
            });
            r.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
            r
        }

        fn shared_tab(&self, panel: &str) -> Rect {
            self.ctx.read_response(Id::new("vectorcraft-panel-docking").with(("tab", &panel.to_string()))).unwrap().rect
        }

        /// The dock's tabs, left to right.
        fn dock_tabs(&self) -> Vec<Rect> {
            if let Some(layout) = &self.app.ui.docking {
                let root = craft_ui::docking::Layout { root: layout.root.clone(), floating: Vec::new() };
                return root
                    .panels()
                    .into_iter()
                    .filter_map(|panel| {
                        self.ctx.read_response(Id::new("vectorcraft-panel-docking").with(("tab", panel))).map(|response| response.rect)
                    })
                    .collect();
            }
            let dock = self.temp_rect(dock_rect_id());
            self.widgets(|r| r.height() == 32.0 && dock.contains_rect(r))
        }

        /// The icon column's panel icons, top to bottom.
        fn icons(&self) -> Vec<Rect> {
            let dock = self.temp_rect(if self.app.ui.docking.is_some() { icons_rect_id() } else { dock_rect_id() });
            self.widgets(|r| r.size() == vec2(30.0, 30.0) && dock.contains_rect(r))
        }

        fn groups(&self) -> Vec<Vec<&str>> {
            self.app.ui.floating_panels.iter().map(|g| g.panels.iter().map(String::as_str).collect()).collect()
        }
    }

    #[test]
    fn floating_group_tabs_reorder_horizontally_within_the_strip() {
        let mut h = Harness::new();
        h.drag(h.icons()[2].center(), pos2(500.0, 250.0));
        h.drag(h.icons()[0].center(), h.shared_tab("swatches").center());
        assert_eq!(h.groups(), [["swatches", "color"]]);
        h.app.run("window.panel.move", json!({ "panel": "color", "anchor": "swatches", "before": "swatches" })).unwrap();
        assert_eq!(h.groups(), [["color", "swatches"]]);
    }

    #[test]
    fn a_dock_tab_dragged_out_floats_and_dropped_on_the_dock_docks_again() {
        let mut h = Harness::new();
        let tabs = h.dock_tabs();
        assert_eq!(tabs.len(), 3);
        let to = pos2(600.0, 400.0);
        h.drag(tabs[1].center(), to);
        assert_eq!(h.groups(), [["layers"]]);
        assert!(h.group("layers").contains(to));
        assert_eq!(h.dock_tabs().len(), 2);
        assert_eq!(h.app.ui.dock_tab, DockTab::Properties);
        h.click(h.title("layers"));
        assert_eq!(h.groups(), [["layers"]], "a click only activates the floating tab");
        let over = h.shared_tab("properties").center();
        h.drag(h.title("layers"), over);
        assert!(h.groups().is_empty());
        assert_eq!((h.dock_tabs().len(), h.app.ui.dock_tab), (3, DockTab::Layers));
    }

    #[test]
    fn the_strip_right_of_the_tabs_floats_the_whole_group_and_its_close_box_docks_it() {
        let mut h = Harness::new();
        let tabs = h.dock_tabs();
        let rest = tabs[2].right_center() + vec2(30.0, 0.0);
        h.drag(rest, pos2(500.0, 300.0));
        assert_eq!(h.groups(), [["properties", "layers", "libraries"]]);
        assert!(h.dock_tabs().is_empty(), "the tabbed group left the dock");
        let column = h.temp_rect(dock_rect_id());
        assert!(column.width() < 40.0, "only the icon column is left: {column:?}");
        // Its × puts them back.
        let g = h.group("properties");
        let close =
            if h.app.ui.docking.is_some() { pos2(g.right() - 36.0, g.top() + 16.0) } else { pos2(g.right() - 11.0, g.top() + 1.0 + TITLE / 2.0) };
        h.click(close);
        assert!(h.groups().is_empty());
        assert_eq!(h.dock_tabs().len(), 3);
    }

    #[test]
    fn icons_dragged_out_float_stack_onto_another_group_and_tear_out_of_it() {
        // Tall enough that the icon column shows every icon, so counting them says what it
        // means: one leaves the column, rather than one more scrolling into view.
        let mut h = Harness::on(vec2(1400.0, 1600.0));
        // Color, Color Guide, Swatches… top to bottom.
        let icons = h.icons();
        let count = icons.len();
        assert_eq!(count, crate::state::ICON_PANEL_GROUPS.iter().map(|g| g.len()).sum::<usize>(), "every group's icons are drawn");
        h.drag(icons[2].center(), pos2(500.0, 250.0));
        assert_eq!(h.groups(), [["swatches"]]);
        assert_eq!(h.icons().len(), count - 1);
        let separate = pos2(900.0, 650.0);
        assert!(!h.group("swatches").contains(separate));
        h.drag(h.icons()[0].center(), separate);
        assert_eq!(h.groups(), [["swatches"], ["color"]]);
        let onto = h.shared_tab("swatches").right_center() - vec2(3.0, 0.0);
        h.drag(h.title("color"), onto);
        assert_eq!(h.groups(), [["swatches", "color"]]);
        assert_eq!(h.app.ui.floating_panels[0].active, 1);
        h.click(h.shared_tab("swatches").center());
        assert_eq!(h.app.ui.floating_panels[0].active, 0);
        let to = pos2(900.0, 500.0);
        h.drag(h.shared_tab("color").center(), to);
        assert_eq!(h.groups(), [["swatches"], ["color"]]);
        assert!(h.group("color").contains(to));
        assert_eq!(h.app.run("window.panel", json!({"panel": "Color"})).unwrap(), json!({"floating": ["color"]}));
        assert_eq!(crate::menus::checked(&h.app, "window.panel", &json!({"panel": "color"})), Some(true));
        h.app.run("window.panel.dock", json!({"panel": "color"})).unwrap();
        h.settle();
        assert!(h.groups().iter().all(|group| !group.contains(&"color")));
        assert_eq!(h.icons().len(), count - 1, "Color is back in the column");
        assert_eq!(h.app.ui.open_panel.as_deref(), Some("color"));
    }

    #[test]
    fn an_icon_panel_can_be_dropped_into_another_docked_tab_group() {
        let mut h = Harness::new();
        h.drag(h.icons()[0].center(), pos2(500.0, 400.0));
        assert_eq!(h.groups(), [["color"]]);
        h.drag(h.shared_tab("color").center(), h.shared_tab("layers").center());
        assert!(h.groups().is_empty());
        assert!(h.app.ui.docking.as_ref().unwrap().contains(&"color".into()));
        assert!(h.app.ui.open_panel.is_none(), "the actual shared group owns Color");
        assert_eq!(h.dock_tabs().len(), 4);
    }

    /// A floating icon panel dropped on the icon column lights it up, goes back and pops out of its icon.
    #[test]
    fn a_panel_dropped_on_the_icon_column_lights_it_up_and_pops_out_of_its_icon() {
        let mut h = Harness::new();
        let count = h.icons().len();
        h.drag(h.icons()[0].center(), pos2(500.0, 400.0));
        assert_eq!(h.groups(), [["color"]]);
        let (dock, column) = (h.temp_rect(dock_rect_id()), h.temp_rect(icons_rect_id()));
        let over = column.center();
        h.hold(h.title("color"), over);
        assert_eq!(drop_target(&h.app, &h.ctx, Moving::Panel("color"), over), Some((Drop::Icons, column)));
        assert!(column.width() < dock.width(), "the column, not the whole dock: {column:?} {dock:?}");
        // Dropped, it's back in the column and pops out of its icon rather than vanishing.
        h.release(over);
        assert!(h.groups().is_empty(), "{}", h.app.ui.status);
        assert_eq!((h.icons().len(), h.app.ui.open_panel.as_deref()), (count, Some("color")));
    }

    /// A floating panel dropped on the dock's tabs joins them; on the group's lower half it makes a group below.
    #[test]
    fn a_panel_dropped_on_the_dock_tabs_joins_them_and_on_its_lower_half_makes_a_group_below() {
        let mut h = Harness::new();
        h.drag(h.icons()[0].center(), pos2(500.0, 400.0));
        assert_eq!(h.groups(), [["color"]]);
        // Over the tabbed group's tabs: they light up, and Color joins them.
        let over = h.dock_tabs()[1].center();
        h.hold(h.title("color"), over);
        assert!(matches!(drop_target(&h.app, &h.ctx, Moving::Panel("color"), over), Some((Drop::Root(_, RootZone::Tab), _))));
        h.release(over);
        assert!(h.groups().is_empty());
        assert_eq!(h.dock_tabs().len(), 4);
        assert!(h.app.ui.open_panel.is_none());
        // Color Guide on the group's lower half: a group of its own below the tabbed group.
        h.drag(h.icons()[0].center(), pos2(500.0, 400.0));
        assert_eq!(h.groups(), [["colorGuide"]]);
        let dock = h.ctx.data(|d| d.get_temp::<Vec<RootGroup>>(root_groups_id())).unwrap()[0].rect;
        let lower = pos2(dock.center().x, dock.bottom() - 60.0);
        h.hold(h.title("colorGuide"), lower);
        assert!(matches!(drop_target(&h.app, &h.ctx, Moving::Panel("colorGuide"), lower), Some((Drop::Root(_, RootZone::Bottom), _))));
        h.release(lower);
        assert!(h.groups().is_empty());
        let groups = h.ctx.data(|d| d.get_temp::<Vec<RootGroup>>(root_groups_id())).unwrap();
        assert_eq!(
            groups.iter().map(|g| g.panels.clone()).collect::<Vec<_>>(),
            [vec!["properties", "layers", "libraries", "color"], vec!["colorGuide"]]
        );
        assert!(groups[0].rect.bottom() <= groups[1].rect.top() + 1.0);
    }

    #[test]
    fn a_popped_out_panel_dragged_by_its_title_floats() {
        let mut h = Harness::new();
        h.app.run("window.panel", json!({"panel": "swatches"})).unwrap();
        h.settle();
        let flyout = h.ctx.memory(|m| m.area_rect(Id::new("icon-panel"))).unwrap();
        let to = pos2(500.0, 400.0);
        h.drag(flyout.left_top() + vec2(20.0, 13.0), to);
        assert_eq!((h.groups(), h.app.ui.open_panel.as_deref()), (vec![vec!["swatches"]], None));
        assert!(h.group("swatches").contains(to), "under the pointer: {:?}", h.group("swatches"));
        // Its panel menu docks it again.
        h.app.run("window.panel.dock", json!({"panel": "swatches"})).unwrap();
        assert!(h.groups().is_empty());
    }

    #[test]
    fn popped_out_title_drag_cancels_on_escape_focus_loss_and_workspace_change() {
        for reason in ["escape", "focus", "workspace"] {
            let mut h = Harness::new();
            h.app.run("window.panel", json!({"panel":"swatches"})).unwrap();
            h.settle();
            let original = h.app.ui.clone();
            let flyout = h.ctx.memory(|m| m.area_rect(Id::new("icon-panel"))).unwrap();
            let to = pos2(500.0, 400.0);
            h.hold(flyout.left_top() + vec2(20.0, 13.0), to);
            assert!(h.app.ui.floating_panels.iter().any(|g| g.panels.contains(&"swatches".into())));
            match reason {
                "escape" => h.frame(vec![Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }]),
                "focus" => h.frame(vec![Event::WindowFocused(false)]),
                _ => {
                    crate::workspaces::apply(&mut h.app.ui, &crate::workspaces::Workspace::default());
                    h.frame(vec![Event::PointerMoved(pos2(650.0, 450.0))]);
                }
            }
            assert!(h.ctx.data(|d| d.get_temp::<Move>(move_id())).is_none(), "{reason}: stale gesture removed");
            assert!(h.app.ui.docking.is_none(), "{reason}: original/selected workspace retained");
            assert!(h.app.ui.floating_panels.is_empty(), "{reason}: no stale floating group");
            if reason != "workspace" {
                assert_eq!(h.app.ui.open_panel, original.open_panel);
            }
            if reason == "focus" {
                h.frame(vec![Event::WindowFocused(true)]);
            }
            for step in 1..=3 {
                h.frame(vec![Event::PointerMoved(to + vec2(15.0 * step as f32, 8.0))]);
                assert!(h.app.ui.floating_panels.is_empty(), "{reason}: canceled drag stays canceled on held frame {step}");
            }
            h.frame(vec![Event::PointerButton { pos: to, button: PointerButton::Secondary, pressed: true, modifiers: Default::default() }]);
            assert!(h.app.ui.floating_panels.is_empty(), "{reason}: secondary press does not restart primary gesture");
            h.frame(vec![Event::PointerButton { pos: to, button: PointerButton::Secondary, pressed: false, modifiers: Default::default() }]);
            h.release(to);
            assert!(h.app.ui.floating_panels.is_empty(), "{reason}: release does not restart canceled gesture");
        }
    }

    #[test]
    fn docked_tools_drag_cancellation_restores_the_docked_origin() {
        let mut h = Harness::new();
        let zone = h.temp_rect(tools_zone_id());
        let title = zone.left_top() + vec2(30.0, 7.0);
        let to = pos2(400.0, 300.0);
        h.hold(title, to);
        assert!(h.app.ui.toolbar_pos.is_some());
        h.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert_eq!(h.app.ui.toolbar_pos, None, "cancel restores docked Tools origin");
        h.frame(vec![Event::PointerMoved(to + vec2(30.0, 20.0))]);
        assert_eq!(h.app.ui.toolbar_pos, None);
        h.release(to);
        assert_eq!(h.app.ui.toolbar_pos, None);
    }

    #[test]
    fn the_tools_panel_floats_by_its_title_bar_and_docks_on_the_left_edge() {
        let mut h = Harness::new();
        let zone = h.temp_rect(tools_zone_id());
        let title = zone.left_top() + vec2(30.0, 7.0);
        let to = pos2(400.0, 300.0);
        h.drag(title, to);
        let pos = h.app.ui.toolbar_pos.expect("the Tools panel floats");
        let float = h.ctx.memory(|m| m.area_rect(Id::new("toolbar-floating"))).unwrap();
        assert!(float.contains(to) && (pos2(pos[0], pos[1]) - float.min).length() < 1.0, "under the pointer: {float:?}");
        // Back on the window's left edge.
        let edge = pos2(10.0, 400.0);
        h.hold(float.left_top() + vec2(30.0, 7.0), edge);
        assert_eq!(drop_target(&h.app, &h.ctx, Moving::Tools, edge).map(|t| t.0), Some(Drop::Dock));
        h.release(edge);
        assert_eq!(h.app.ui.toolbar_pos, None, "docked");
        // A click on the title bar still switches the columns.
        let double = h.app.ui.toolbar_double;
        h.click(title);
        assert_ne!(h.app.ui.toolbar_double, double);
    }

    #[test]
    fn floating_panels_are_kept_in_workspaces_and_preferences_and_clamped_into_the_window() {
        let mut h = Harness::new();
        let app = &mut h.app;
        // Saved on a much bigger screen.
        app.run("window.panel.float", json!({"panel": "layers", "x": 5000, "y": 4000})).unwrap();
        let r = app.run("window.panel.float", json!({"panel": "stroke", "onto": "Layers"})).unwrap();
        assert_eq!(r["group"], json!(["layers", "stroke"]));
        app.run("window.panel.float", json!({"panel": "tools", "x": 300, "y": 200})).unwrap();
        app.run("window.workspace.new", json!({"name": "Floating"})).unwrap();
        let saved = app.ui.floating_panels.clone();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(app.ui.floating_panels.is_empty() && app.ui.toolbar_pos.is_none(), "Essentials docks everything");
        app.run("window.workspace", json!({"name": "Floating"})).unwrap();
        assert_eq!((&app.ui.floating_panels, app.ui.toolbar_pos), (&saved, Some([300.0, 200.0])));
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        let prefs = prefs.sanitized();
        assert_eq!((&prefs.floating_panels, prefs.toolbar_pos), (&saved, Some([300.0, 200.0])));
        // Unknown and repeated panels (a hand-edited file) are dropped.
        let edited: UiState = serde_json::from_value(json!({
            "floating_panels": [{"panels": ["nope", "layers"], "active": 7, "pos": [1, 2]}, {"panels": ["layers"], "pos": [3, 4]}],
        }))
        .unwrap();
        let edited = edited.sanitized();
        assert_eq!(edited.floating_panels, [FloatingPanels { panels: vec!["layers".into()], active: 0, pos: [1.0, 2.0], ..Default::default() }]);
        h.settle();
        let g = h.group("layers");
        let screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
        assert!(screen.contains_rect(g), "clamped into the window: {g:?}");
        assert!(h.dock_tabs().len() == 2 && h.app.ui.dock_tab == DockTab::Properties);
    }

    #[test]
    fn float_and_dock_commands_check_their_params() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(app.run("window.panel.float", json!({"panel": "nope"})).is_err());
        assert!(app.run("window.panel.float", json!({"panel": "layers", "x": "left", "y": 2})).is_err());
        assert!(app.run("window.panel.float", json!({"panel": "layers", "x": 2})).is_err(), "both x and y");
        assert!(app.run("window.panel.float", json!({"panel": "layers", "group": "yes"})).is_err());
        assert!(app.run("window.panel.float", json!({"panel": "layers", "onto": "color"})).is_err(), "Color isn't floating");
        assert!(app.ui.floating_panels.is_empty());
        // The tabbed group's tabs float together.
        let r = app.run("window.panel.float", json!({"panel": "layers", "group": true})).unwrap();
        assert_eq!(r["group"], json!(["properties", "layers", "libraries"]));
        assert_eq!(app.ui.floating_panels[0].active, 1);
        assert!(app.run("window.panel.float", json!({"panel": "layers", "onto": "properties"})).is_err(), "not onto its own group");
        // Floated out of its group; a lone panel moves.
        app.run("window.panel.float", json!({"panel": "layers", "x": 10, "y": 20})).unwrap();
        assert_eq!((app.ui.floating_panels.len(), floating_pos(&app, "layers")), (2, [10.0, 20.0]));
        app.run("window.panel.float", json!({"panel": "layers", "x": 30, "y": 40})).unwrap();
        assert_eq!((app.ui.floating_panels.len(), floating_pos(&app, "layers")), (2, [30.0, 40.0]));
        app.run("window.panel.dock", json!({"panel": "properties", "group": true})).unwrap();
        assert_eq!(app.ui.floating_panels.len(), 1);
        assert_eq!(app.ui.dock_tab, DockTab::Properties);
        assert_eq!(app.run("window.panel.dock", json!({"panel": "tools"})).unwrap()["floating"], json!(false));
        app.ui.toolbar = false;
        app.run("window.panel.float", json!({"panel": "Toolbar"})).unwrap();
        assert!(app.ui.toolbar && app.ui.toolbar_pos == Some(TOOLS_AT), "floating shows the Tools panel");
    }

    /// The groups' sets: each group's panels and its column, in list order.
    fn sets(groups: &[FloatingPanels]) -> Vec<(Vec<&str>, Option<u32>)> {
        groups.iter().map(|g| (g.panels.iter().map(String::as_str).collect(), g.column)).collect()
    }

    #[test]
    fn groups_dropped_on_an_edge_stack_into_a_set_that_moves_together_and_tears_apart() {
        let mut h = Harness::new();
        h.app.run("window.panel.float", json!({"panel": "color", "x": 500, "y": 150})).unwrap();
        h.app.run("window.panel.float", json!({"panel": "swatches", "x": 950, "y": 450})).unwrap();
        h.settle();
        // Swatches' title bar held over Color's bottom edge: a line shows it goes below.
        let color = h.temp_rect(group_rect_id("color"));
        let below = pos2(color.center().x, color.bottom() + 2.0);
        h.hold(h.title("swatches"), below);
        let target = drop_target(&h.app, &h.ctx, Moving::Panel("swatches"), below);
        assert_eq!(target.map(|t| t.0), Some(Drop::Below("color")));
        assert!(target.is_some_and(|t| t.1.height() <= 4.0), "a line, not a box: {target:?}");
        h.release(below);
        let s = sets(&h.app.ui.floating_panels);
        assert_eq!((s[0].0.clone(), s[1].0.clone()), (vec!["color"], vec!["swatches"]), "Color, then Swatches");
        assert!(s[0].1.is_some() && s[0].1 == s[1].1, "one set: {s:?}");
        assert_eq!(h.app.ui.floating_panels[0].pos, h.app.ui.floating_panels[1].pos, "at the set's corner");
        // Drawn as one box: Swatches right under Color.
        let (color, swatches) = (h.temp_rect(group_rect_id("color")), h.temp_rect(group_rect_id("swatches")));
        assert!((swatches.top() - color.bottom()).abs() < 1.0 && swatches.left() == color.left(), "{color:?} {swatches:?}");
        assert!(h.group("color").contains_rect(swatches.shrink(1.0)), "in Color's area");
        // The set moves by its title bar.
        let to = pos2(300.0, 120.0);
        let grab = h.title("color") - h.group("color").min;
        h.drag(h.title("color"), to);
        let moved = [to.x - grab.x, to.y - grab.y];
        assert!(h.app.ui.floating_panels.iter().all(|g| (g.pos[0] - moved[0]).abs() < 1.0 && (g.pos[1] - moved[1]).abs() < 1.0));
        assert_eq!(sets(&h.app.ui.floating_panels).len(), 2);
        // Swatches' lone tab dragged away takes it out of the set: both float on their own.
        let swatches = h.temp_rect(group_rect_id("swatches"));
        let tab = pos2(swatches.left() + 20.0, swatches.top() + GAP + STRIP / 2.0);
        let away = pos2(1000.0, 600.0);
        h.drag(tab, away);
        assert_eq!(sets(&h.app.ui.floating_panels), [(vec!["color"], None), (vec!["swatches"], None)]);
        assert!(h.group("swatches").contains(away), "under the pointer: {:?}", h.group("swatches"));
        // Dropped on Color's top edge, it goes above Color, which it now heads.
        let color = h.temp_rect(group_rect_id("color"));
        let above = pos2(color.center().x, color.top() - 3.0);
        h.hold(h.title("swatches"), above);
        assert_eq!(drop_target(&h.app, &h.ctx, Moving::Panel("swatches"), above).map(|t| t.0), Some(Drop::Above("color")));
        h.release(above);
        let s = sets(&h.app.ui.floating_panels);
        assert_eq!((s[0].0.clone(), s[1].0.clone()), (vec!["swatches"], vec!["color"]));
        assert!(s[0].1.is_some() && s[0].1 == s[1].1);
        // A group's tab strip still takes tabs: Stroke dropped on Color's tabs joins Color's group.
        h.app.run("window.panel.float", json!({"panel": "stroke", "x": 1000, "y": 650})).unwrap();
        h.settle();
        let color = h.temp_rect(group_rect_id("color"));
        let tabs = pos2(color.center().x, color.top() + GAP + STRIP / 2.0);
        h.drag(h.title("stroke"), tabs);
        assert_eq!(sets(&h.app.ui.floating_panels).iter().map(|s| s.0.clone()).collect::<Vec<_>>(), [vec!["swatches"], vec!["color", "stroke"]]);
        // The set's × docks every group.
        let g = h.group("swatches");
        h.click(pos2(g.right() - 11.0, g.top() + 1.0 + TITLE / 2.0));
        assert!(h.groups().is_empty(), "docked: {:?}", h.groups());
    }

    #[test]
    fn a_double_click_on_a_tab_collapses_a_floating_group_and_another_expands_it() {
        let mut h = Harness::new();
        h.app.run("window.panel.float", json!({"panel": "layers", "x": 500, "y": 150})).unwrap();
        h.app.run("window.panel.float", json!({"panel": "stroke", "below": "layers"})).unwrap();
        h.settle();
        let open = h.temp_rect(group_rect_id("layers"));
        let tab = pos2(open.left() + 20.0, open.top() + 1.0 + TITLE + STRIP / 2.0);
        let double = |h: &mut Harness| {
            // Well after any earlier click, so the first click doesn't make a double-click with it.
            h.time += 1.0;
            h.frame(vec![Event::PointerMoved(tab), button(tab, true)]);
            h.frame(vec![button(tab, false)]);
            h.frame(vec![button(tab, true)]);
            h.frame(vec![button(tab, false)]);
            h.settle();
        };
        double(&mut h);
        assert!(h.app.ui.floating_panels[0].collapsed, "collapsed");
        let shut = h.temp_rect(group_rect_id("layers"));
        assert!(shut.height() < 1.0 + TITLE + STRIP + 2.0, "only its tabs: {shut:?}");
        assert!(h.temp_rect(group_rect_id("stroke")).top() < open.bottom(), "Stroke moved up under it");
        double(&mut h);
        assert!(!h.app.ui.floating_panels[0].collapsed, "expanded");
        // Collapsed by command, Window › Layers expands it.
        h.app.run("window.panel.float", json!({"panel": "layers", "collapsed": true})).unwrap();
        assert!(h.app.ui.floating_panels[0].collapsed);
        h.app.run("window.panel", json!({"panel": "layers"})).unwrap();
        assert!(!h.app.ui.floating_panels[0].collapsed);
    }

    /// Illustrator also collapses the group from the blank part of the strip, see
    /// <https://helpx.adobe.com/illustrator/using/customizing-workspace.html>.
    #[test]
    fn a_double_click_on_the_strip_right_of_the_tabs_collapses_a_floating_group_in_place() {
        let mut h = Harness::new();
        h.app.run("window.panel.float", json!({"panel": "layers", "x": 500, "y": 150})).unwrap();
        h.app.run("window.panel.float", json!({"panel": "stroke", "below": "layers"})).unwrap();
        h.settle();
        let owned = |h: &Harness| format!("{:?}", sets(&h.app.ui.floating_panels));
        let before = owned(&h);
        let open = h.temp_rect(group_rect_id("layers"));
        // Left of the panel menu, well right of the lone Layers tab.
        let blank = pos2(open.right() - 40.0, open.top() + 1.0 + TITLE + STRIP / 2.0);
        let double = |h: &mut Harness| {
            h.time += 1.0;
            h.frame(vec![Event::PointerMoved(blank), button(blank, true)]);
            h.frame(vec![button(blank, false)]);
            h.frame(vec![button(blank, true)]);
            h.frame(vec![button(blank, false)]);
            h.settle();
        };
        double(&mut h);
        assert!(h.app.ui.floating_panels[0].collapsed, "collapsed");
        assert_eq!(owned(&h), before, "still in its set");
        assert_eq!(h.temp_rect(group_rect_id("layers")).min, open.min, "not moved");
        double(&mut h);
        assert!(!h.app.ui.floating_panels[0].collapsed, "expanded");
    }

    #[test]
    fn sets_by_command_are_checked_and_kept_in_workspaces_and_preferences() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("window.panel.float", json!({"panel": "layers", "x": 100, "y": 50})).unwrap();
        assert!(app.run("window.panel.float", json!({"panel": "color", "below": "swatches"})).is_err(), "Swatches isn't floating");
        assert!(app.run("window.panel.float", json!({"panel": "color", "onto": "layers", "below": "layers"})).is_err(), "one of them");
        assert!(app.run("window.panel.float", json!({"panel": "layers", "below": "layers"})).is_err(), "not below itself");
        assert!(app.run("window.panel.float", json!({"panel": "layers", "collapsed": "yes"})).is_err());
        assert_eq!(app.ui.floating_panels.len(), 1, "nothing floated by the errors");
        // Color floats straight into the set, below Layers; Stroke above it.
        let r = app.run("window.panel.float", json!({"panel": "color", "below": "Layers"})).unwrap();
        assert_eq!(r["set"], json!([["layers"], ["color"]]));
        let r = app.run("window.panel.float", json!({"panel": "stroke", "above": "color"})).unwrap();
        assert_eq!(r["set"], json!([["layers"], ["stroke"], ["color"]]));
        assert!(app.ui.floating_panels.iter().all(|g| g.pos == [100.0, 50.0]), "at the set's corner");
        // Stroke's whole group moves to the set's bottom.
        app.run("window.panel.float", json!({"panel": "stroke", "below": "color"})).unwrap();
        assert_eq!(sets(&app.ui.floating_panels).iter().map(|s| s.0.clone()).collect::<Vec<_>>(), [vec!["layers"], vec!["color"], vec!["stroke"]]);
        // Kept in a workspace and the preferences.
        app.run("window.panel.float", json!({"panel": "color", "collapsed": true})).unwrap();
        app.run("window.workspace.new", json!({"name": "Sets"})).unwrap();
        let saved = app.ui.floating_panels.clone();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(app.ui.floating_panels.is_empty());
        app.run("window.workspace", json!({"name": "Sets"})).unwrap();
        assert_eq!(app.ui.floating_panels, saved);
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        assert_eq!(prefs.sanitized().floating_panels, saved);
        // Moved to x, y, a group leaves its set; the set left with one group floats on its own.
        let r = app.run("window.panel.float", json!({"panel": "color", "x": 600, "y": 400})).unwrap();
        assert_eq!((r["set"].clone(), r["pos"].clone()), (json!([["color"]]), json!([600.0, 400.0])));
        app.run("window.panel.dock", json!({"panel": "layers"})).unwrap();
        assert_eq!(sets(&app.ui.floating_panels), [(vec!["stroke"], None), (vec!["color"], None)]);
        // A hand-edited file: a set's groups kept together at its first group's corner, a lone one alone.
        let edited: UiState = serde_json::from_value(json!({
            "floating_panels": [
                {"panels": ["layers"], "pos": [1, 2], "column": 4},
                {"panels": ["color"], "pos": [9, 9]},
                {"panels": ["stroke"], "pos": [5, 6], "column": 4},
                {"panels": ["swatches"], "pos": [7, 8], "column": 9},
            ],
        }))
        .unwrap();
        let edited = edited.sanitized();
        let s = sets(&edited.floating_panels);
        assert_eq!(s, [(vec!["layers"], Some(4)), (vec!["stroke"], Some(4)), (vec!["color"], None), (vec!["swatches"], None)]);
        assert_eq!(edited.floating_panels[1].pos, [1.0, 2.0]);
    }

    /// The dock columns as drawn, next to the icon column first.
    fn columns(h: &Harness) -> Vec<Rect> {
        h.ctx.data(|d| d.get_temp::<Vec<Rect>>(columns_id())).unwrap_or_default()
    }

    /// Each group's panels and dock column, in list order.
    fn docked_of(groups: &[FloatingPanels]) -> Vec<(Vec<&str>, Option<u32>)> {
        groups.iter().map(|g| (g.panels.iter().map(String::as_str).collect(), g.docked)).collect()
    }

    #[test]
    fn sets_dropped_on_the_docks_left_edge_become_dock_columns() {
        let mut h = Harness::new();
        h.app.run("window.panel.float", json!({"panel": "color", "x": 300, "y": 150})).unwrap();
        h.settle();
        // Held over the icon column's left edge, a line shows the new column; dropped, it docks.
        let icons = h.temp_rect(icons_rect_id());
        let edge = pos2(icons.left() - 2.0, icons.center().y);
        h.hold(h.title("color"), edge);
        let target = drop_target(&h.app, &h.ctx, Moving::Panel("color"), edge);
        assert_eq!(target.map(|t| t.0), Some(Drop::Column(1)));
        assert!(target.is_some_and(|t| t.1.width() <= 4.0), "a line: {target:?}");
        h.release(edge);
        assert_eq!(docked_of(&h.app.ui.floating_panels), [(vec!["color"], Some(1))]);
        let cols = columns(&h);
        assert_eq!(cols.len(), 1);
        let icons = h.temp_rect(icons_rect_id());
        assert!((cols[0].right() - icons.left()).abs() < 2.0, "right of it the icon column: {cols:?} {icons:?}");
        // Drawn in the column, not floating too (a floating box, drawn after the dock, would move it).
        assert!(cols[0].contains_rect(h.temp_rect(group_rect_id("color")).shrink(1.0)), "Color in the column");
        // Swatches dropped on the docked Color's bottom edge joins its column.
        h.app.run("window.panel.float", json!({"panel": "swatches", "x": 300, "y": 300})).unwrap();
        h.settle();
        let color = h.temp_rect(group_rect_id("color"));
        let below = pos2(color.center().x, color.bottom() + 2.0);
        h.drag(h.title("swatches"), below);
        assert_eq!(docked_of(&h.app.ui.floating_panels), [(vec!["color"], Some(1)), (vec!["swatches"], Some(1))]);
        assert!(columns(&h)[0].contains_rect(h.temp_rect(group_rect_id("swatches")).shrink(1.0)));
        // Stroke dropped on the column's left edge makes a second column, left of it.
        h.app.run("window.panel.float", json!({"panel": "stroke", "x": 300, "y": 400})).unwrap();
        h.settle();
        let col = columns(&h)[0];
        h.drag(h.title("stroke"), pos2(col.left() - 2.0, col.center().y));
        assert_eq!(docked_of(&h.app.ui.floating_panels).last(), Some(&(vec!["stroke"], Some(2))));
        let cols = columns(&h);
        assert!(cols.len() == 2 && cols[1].right() <= cols[0].left() + 2.0, "{cols:?}");
        // A docked lone tab dragged away floats its group, leaving the rest of the column docked.
        let swatches = h.temp_rect(group_rect_id("swatches"));
        h.drag(pos2(swatches.left() + 20.0, swatches.top() + GAP + STRIP / 2.0), pos2(500.0, 500.0));
        let d = docked_of(&h.app.ui.floating_panels);
        assert!(d.contains(&(vec!["swatches"], None)) && d.contains(&(vec!["color"], Some(1))), "{d:?}");
        // The column's top strip floats the whole column.
        let col = columns(&h)[0];
        h.drag(pos2(col.left() + 30.0, col.top() + TITLE / 2.0), pos2(500.0, 200.0));
        let d = docked_of(&h.app.ui.floating_panels);
        assert!(d.contains(&(vec!["color"], None)) && d.contains(&(vec!["stroke"], Some(1))), "renumbered: {d:?}");
        assert!(h.group("color").contains(pos2(500.0, 200.0)), "under the pointer");
        // The column's × puts its panels back in the dock.
        let col = columns(&h)[0];
        h.click(pos2(col.right() - 11.0, col.top() + TITLE / 2.0));
        assert!(!h.app.ui.floating_panels.iter().any(|g| g.panels.contains(&"stroke".to_string())));
        assert!(columns(&h).is_empty());
    }

    #[test]
    fn dock_columns_by_command_are_checked_and_kept_in_workspaces() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(app.run("window.panel.dock", json!({"panel": "color", "column": 0})).is_err());
        assert!(app.run("window.panel.dock", json!({"panel": "color", "column": "left"})).is_err());
        assert!(app.ui.floating_panels.is_empty());
        let r = app.run("window.panel.dock", json!({"panel": "color", "column": true})).unwrap();
        assert_eq!((r["column"].clone(), r["floating"].clone()), (json!(1), json!(false)));
        app.run("window.panel.dock", json!({"panel": "layers", "column": true})).unwrap();
        // Next to the icon column, moving Color and Layers one column left.
        app.run("window.panel.dock", json!({"panel": "stroke", "column": 1})).unwrap();
        app.run("window.panel.float", json!({"panel": "gradient", "below": "stroke"})).unwrap();
        let d = docked_of(&app.ui.floating_panels);
        for (panel, column) in [("stroke", 1), ("gradient", 1), ("color", 2), ("layers", 3)] {
            assert!(d.contains(&(vec![panel], Some(column))), "{panel} in column {column}: {d:?}");
        }
        // Kept in a workspace and the preferences.
        app.run("window.workspace.new", json!({"name": "Columns"})).unwrap();
        let saved = app.ui.floating_panels.clone();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        app.run("window.workspace", json!({"name": "Columns"})).unwrap();
        assert_eq!(app.ui.floating_panels, saved);
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        assert_eq!(prefs.sanitized().floating_panels, saved);
        // Floated, Color's column leaves the dock and the columns close up.
        let r = app.run("window.panel.float", json!({"panel": "color"})).unwrap();
        assert_eq!((r["floating"].clone(), r["column"].clone()), (json!(true), json!(null)));
        assert!(docked_of(&app.ui.floating_panels).contains(&(vec!["layers"], Some(2))));
        // A hand-edited file: columns numbered without gaps, a set's groups in its first one's.
        let edited: UiState = serde_json::from_value(json!({
            "floating_panels": [
                {"panels": ["layers"], "pos": [1, 2], "docked": 7},
                {"panels": ["color"], "pos": [1, 2], "column": 3, "docked": 3},
                {"panels": ["stroke"], "pos": [5, 6], "column": 3},
            ],
        }))
        .unwrap();
        let edited = edited.sanitized();
        let d = docked_of(&edited.floating_panels);
        assert_eq!(d, [(vec!["layers"], Some(2)), (vec!["color"], Some(1)), (vec!["stroke"], Some(1))]);
    }

    #[test]
    fn a_dock_column_takes_groups_anywhere_in_it_and_as_tabs() {
        let mut h = Harness::new();
        h.app.run("window.panel.dock", json!({"panel": "color", "column": true})).unwrap();
        h.app.run("window.panel.float", json!({"panel": "swatches", "below": "color"})).unwrap();
        h.app.run("window.panel.float", json!({"panel": "stroke", "below": "swatches"})).unwrap();
        h.settle();
        let order = |h: &Harness| h.app.ui.floating_panels.iter().map(|g| g.panels.join("+")).collect::<Vec<_>>();
        // Dropped on the upper half of Swatches, Gradient goes in right above it.
        h.app.run("window.panel.float", json!({"panel": "gradient", "x": 300, "y": 200})).unwrap();
        h.settle();
        let r = h.temp_rect(group_rect_id("swatches"));
        let tabs_bottom = r.top() + GAP + STRIP;
        let upper = pos2(r.center().x, tabs_bottom + (r.bottom() - tabs_bottom) / 4.0);
        h.hold(h.title("gradient"), upper);
        assert_eq!(drop_target(&h.app, &h.ctx, Moving::Panel("gradient"), upper).map(|t| t.0), Some(Drop::Above("swatches")));
        h.release(upper);
        assert_eq!(order(&h), ["color", "gradient", "swatches", "stroke"]);
        assert!(h.app.ui.floating_panels.iter().all(|g| g.docked == Some(1)), "all in the column");
        // Dropped on Stroke's tabs, Transparency joins Stroke's group as a tab (Color and Gradient
        // collapsed, so Stroke is in view in the column, which scrolls).
        for g in h.app.ui.floating_panels.iter_mut().take(2) {
            g.collapsed = true;
        }
        h.app.run("window.panel.float", json!({"panel": "transparency", "x": 300, "y": 200})).unwrap();
        h.settle();
        let r = h.temp_rect(group_rect_id("stroke"));
        let tabs = pos2(r.left() + 120.0, r.top() + GAP + STRIP / 2.0);
        h.hold(h.title("transparency"), tabs);
        assert_eq!(drop_target(&h.app, &h.ctx, Moving::Panel("transparency"), tabs).map(|t| t.0), Some(Drop::Stack("stroke")));
        h.release(tabs);
        assert_eq!(order(&h), ["color", "gradient", "swatches", "stroke+transparency"]);
        // Dropped in the room under the groups (three collapsed to make some), Align goes at the
        // column's bottom.
        for g in h.app.ui.floating_panels.iter_mut().take(3) {
            g.collapsed = true;
        }
        h.app.run("window.panel.float", json!({"panel": "align", "x": 300, "y": 200})).unwrap();
        h.settle();
        let col = columns(&h)[0];
        h.drag(h.title("align"), pos2(col.center().x, col.bottom() - 6.0));
        assert_eq!(order(&h), ["color", "gradient", "swatches", "stroke+transparency", "align"]);
        assert!(h.app.ui.floating_panels.iter().all(|g| g.docked == Some(1)));
    }

    #[test]
    fn a_dock_column_too_tall_for_the_window_scrolls() {
        let mut h = Harness::new();
        h.app.run("window.panel.dock", json!({"panel": "layers", "column": true})).unwrap();
        let mut above = "layers";
        for p in ["properties", "color", "swatches", "stroke", "gradient", "transparency", "align"] {
            h.app.run("window.panel.float", json!({"panel": p, "below": above})).unwrap();
            above = p;
        }
        h.settle();
        let col = columns(&h)[0];
        assert!(col.bottom() <= SCREEN.y + 1.0, "the column stays in the window: {col:?}");
        let last = h.temp_rect(group_rect_id("align"));
        assert!(last.top() > col.bottom(), "Align starts below the window: {last:?} {col:?}");
        // The wheel over the column (Color's tabs, outside any panel's own scrolling) scrolls it,
        // bringing Align up.
        let color = h.temp_rect(group_rect_id("color"));
        let over = pos2(color.left() + 150.0, color.top() + GAP + STRIP / 2.0);
        for _ in 0..4 {
            h.frame(crate::toolbar::tests::wheel(over, 300.0));
        }
        h.settle();
        let moved = h.temp_rect(group_rect_id("align"));
        assert!(moved.top() < last.top() - 100.0, "scrolled up: {moved:?} from {last:?}");
    }

    fn floating_pos(app: &VectorcraftApp, id: &str) -> [f32; 2] {
        group_of(&app.ui, id).map(|i| app.ui.floating_panels[i].pos).unwrap()
    }
}
