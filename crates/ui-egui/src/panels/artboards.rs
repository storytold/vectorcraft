//! Artboards panel: a `PageList` of the document's artboards. Its rows rename in place (a double-click on
//! the name, F2, or the panel menu's Rename; while several artboards are selected, the name typed goes to
//! all of them, numbered), its row menu (right-click) has a Rename that opens the Rename dialog for the
//! selected artboards, and its footer moves a row up or down, adds one (Alt: Artboard Options for the new
//! artboard first), deletes, or opens Rearrange All Artboards; a row dropped on New Artboard duplicates its
//! artboard (with its art when the Artboard tool's Move/Copy Artwork with Artboard is on). The highlighted
//! row is the active artboard, the one the status bar's navigator shows and Fit Artboard in Window fits:
//! clicking a row makes it active, double-clicking its number also fits it in the window. Shift-click selects
//! a range of artboards and Ctrl/Cmd-click adds or removes one, with any tool: another tool gives way to the
//! Artboard tool first (#5698). The options icon at a row's right end opens Artboard Options: a click on the
//! active artboard's icon, or a double-click on another row's icon. The lock left of it shows on a locked
//! artboard, and a click there locks or unlocks the artboard (`artboard.setProps` with `locked`); Delete
//! leaves locked artboards.

use craft::behavior::{EditEvent, IconId, Selection, SelectionChange};
use craft::pages::ui::list::{PageList, PageListEvent, PageListRow};
use craft::ui::buttons::IconButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Outcome as _, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::menu::{Menu, PanelRow};
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::ColorMode;
use vectorcraft_geom::Rect;

use crate::VectorcraftApp;
use crate::chrome::rank;
use craft::keymap::CommandRef;

/// The panel's view state, kept in `UiState::artboards_ui`.
#[derive(Clone, Debug, Default)]
pub struct ArtboardsUi {
    /// The row a Shift-click ranges from, with the uid of the document it belongs to: another document's
    /// anchor is ignored.
    pub anchor: Option<(u64, usize)>,
    /// The row whose name field opens at the list's next pass (the panel menu's Rename); taken by [`show`].
    pub rename: Option<usize>,
    /// The window and its pass in which key focus was last in the list: while the list keeps it, Select All's keys
    /// select every artboard ([`selects_all`]).
    pub focused_pass: Option<(egui::ViewportId, u64)>,
    /// The row whose context menu is open.
    pub menu: Option<usize>,
}

/// The Artboard tool's Move Artwork with Artboard and Scale Artwork with Artboard options (#602) as
/// check boxes: the Control bar ([`control_bar`]) and Properties show them while the tool is in use.
pub(crate) fn art_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    let opts = app.session.tool_options();
    for (key, label, default) in [("moveArt", tl!("Move Artwork with Artboard"), true), ("scaleArt", tl!("Scale Artwork with Artboard"), false)] {
        let on = opts[key].as_bool().unwrap_or(default);
        if let Some(next) = Checkbox::new(Bind::given(on)).label(label).show(ui).events.landed().cloned().filter(|next| *next != on) {
            app.run("tool.setOption", json!({ "key": key, "value": next })).ok();
        }
    }
}

/// An icon button of the Control bar → whether it was clicked.
fn icon_button(ui: &mut Ui, icon: &'static str, label: &'static str, enabled: bool) -> bool {
    IconButton::new(IconId::new(icon)).label(label).enabled(enabled).show(ui).response().clicked()
}

/// The Control bar's Name field showing `name` → the name typed, trimmed, when it is new. An empty field
/// keeps the old name, as in the Properties panel; names are never translated.
fn name_field(ui: &mut Ui, name: &str) -> Option<String> {
    let mut typed = None;
    Field::new(tl!("Name:")).id_salt("ab-name").show(ui, |ui, _| {
        typed = TextField::new(tl!("Name:"), Bind::given(name.to_owned())).show(ui).events.landed().cloned();
    });
    typed.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty() && n != name)
}

/// The Control bar while the Artboard tool is in use, in place of the object controls, one row per group so
/// that each group leaves the bar on its own rank: the active artboard's preset and orientation, New
/// Artboard, Delete Artboard and its name; Artboard Options and Rearrange All Artboards; the Transform link
/// (the Transform panel in a popover); Move and Scale Artwork with Artboard; and the reference point with the
/// artboard's X, Y, W and H. Each control runs the command its Properties panel or Transform panel
/// counterpart runs. With `options` false (the bar's panel menu has its Artboard Options group off), the
/// groups other than the Transform link and the fields are left out; with `transform` false (its Transform
/// group off), the link and the fields are. Returns false, and draws nothing, while another tool is in use.
pub(crate) fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui, options: bool, transform: bool) -> bool {
    if app.session.tool_id() != "artboard" {
        return false;
    }
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let i = selected(app, n);
    let Some(ab) = app.session.active().and_then(|d| d.doc.artboards.get(i)).cloned() else { return true };
    let r = ab.rect;
    if options {
        Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| {
            super::properties::artboard_size_row(app, ui, i, r.width(), r.height());
            if icon_button(ui, "dc-new-item", tl!("New Artboard"), true) {
                add_new(app);
            }
            if icon_button(ui, "trash-2", tl!("Delete Artboard"), n > 1) {
                delete_selected(app);
            }
            if let Some(name) = name_field(ui, &ab.name) {
                app.run("artboard.setProps", json!({"index": i, "name": name})).ok();
            }
        });
        Row::new().fit_rank(rank::DETAIL).show(ui, |ui, _| {
            Divider::vertical().show(ui);
            if icon_button(ui, "dc-artboard-options", tl!("Artboard Options"), true) {
                app.run("tool.options", json!({"tool": "artboard"})).ok();
            }
            if icon_button(ui, "dc-rearrange", tl!("Rearrange All Artboards"), n > 1) {
                open_rearrange(app);
            }
        });
    }
    if transform {
        Row::new().show(ui, |ui, _| {
            if options {
                Divider::vertical().show(ui);
            }
            super::transform::link(app, ui);
        });
    }
    if options {
        Row::new().fit_rank(rank::DETAIL).show(ui, |ui, _| {
            Divider::vertical().show(ui);
            art_options(app, ui);
        });
    }
    if transform {
        Row::new().fit_rank(rank::FIELDS).show(ui, |ui, _| {
            Divider::vertical().show(ui);
            super::transform::artboard_bar_fields(app, ui, i, r);
        });
    }
    true
}

/// Whether artboards resized while the Artboard tool is in use take their art along (its Scale
/// Artwork with Artboard option): the panels' sizes pass it to `artboard.setProps` as `scaleArt`.
pub(crate) fn scale_art(app: &VectorcraftApp) -> bool {
    app.session.tool_id() == "artboard" && app.session.tool_options()["scaleArt"].as_bool().unwrap_or(false)
}

/// Whether position edits take their art along, matching the Artboard tool's move option. The tool
/// keeps the option while another tool is in use, as when this panel opens Artboard Options.
pub(crate) fn move_art(app: &VectorcraftApp) -> bool {
    app.session.tool_options_of("artboard")["moveArt"].as_bool().unwrap_or(true)
}

/// The Fill list of an artboard (Properties › Artboards › Fill, Artboard Options): no background
/// color, White and Black.
pub(crate) const FILLS: [&str; 3] = ["None", "White", "Black"];

/// The background colors [`FILLS`] stand for, White and Black in the document's color mode.
pub(crate) fn fill_colors(app: &VectorcraftApp) -> [Option<Color>; 3] {
    if app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk) {
        [None, Some(Color::cmyk(0.0, 0.0, 0.0, 0.0)), Some(Color::cmyk(0.0, 0.0, 0.0, 1.0))]
    } else {
        [None, Some(Color::WHITE), Some(Color::BLACK)]
    }
}

/// The name the Fill list shows for background `color`: one of [`FILLS`], else Custom.
pub(crate) fn fill_name(app: &VectorcraftApp, color: Option<Color>) -> &'static str {
    fill_colors(app).iter().position(|c| *c == color).map_or("Custom", |i| FILLS[i])
}

/// The active artboard (of `n`).
pub(crate) fn selected(app: &VectorcraftApp, n: usize) -> usize {
    app.view().map_or(0, |v| v.artboard).min(n.saturating_sub(1))
}

/// The selected artboards (of `n`): while the Artboard tool is in use, those it has selected
/// (Shift-click, #1139), else the active one.
pub(crate) fn picked(app: &VectorcraftApp, n: usize) -> Vec<usize> {
    let mut all = tool_selection(app, n);
    if all.is_empty() {
        all.push(selected(app, n));
    }
    all
}

/// The artboards (of `n`) the Artboard tool has selected while it is in use, sorted and once
/// each; empty with another tool or with none selected.
pub(crate) fn tool_selection(app: &VectorcraftApp, n: usize) -> Vec<usize> {
    let tool = (app.session.tool_id() == "artboard").then(|| app.session.tool_options()["selected"].clone());
    let mut all: Vec<usize> = tool
        .iter()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|i| usize::try_from(i).ok())
        .filter(|i| *i < n)
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

/// The picked artboards that can be deleted: those that aren't locked.
pub(crate) fn deletable(app: &VectorcraftApp, n: usize) -> Vec<usize> {
    let locked = |i: &usize| app.session.active().and_then(|d| d.doc.artboards.get(*i)).is_some_and(|a| a.locked);
    picked(app, n).into_iter().filter(|i| !locked(i)).collect()
}

/// The artboards a name typed in artboard `i`'s row's name field goes to: the selected artboards
/// ([`picked`]) when `i` is one of them, else `i` alone. Locked artboards are renamed too.
pub(crate) fn renamed_with(app: &VectorcraftApp, i: usize) -> Vec<usize> {
    let all = picked(app, app.session.active().map_or(0, |d| d.doc.artboards.len()));
    if all.contains(&i) { all } else { vec![i] }
}

/// The artboards a new size for artboard `i` goes to: those [`renamed_with`] gives. Another
/// selected artboard that is locked keeps its size.
pub(crate) fn resized_with(app: &VectorcraftApp, i: usize) -> Vec<usize> {
    let locked = |j: &usize| app.session.active().and_then(|d| d.doc.artboards.get(*j)).is_some_and(|a| a.locked);
    renamed_with(app, i).into_iter().filter(|j| *j == i || !locked(j)).collect()
}

/// The `artboard.setProps` params that set `entries` (each an artboard's `index` and its fields)
/// with `common` (an object: `scaleArt`, `moveArt`), in one undo step: the one entry's fields
/// beside `common`, or an `artboards` list of several beside it; None for no entry.
pub(crate) fn set_props_params(common: Value, mut entries: Vec<Value>) -> Option<Value> {
    let mut p = common;
    let o = p.as_object_mut()?;
    match entries.len() {
        0 => return None,
        1 => {
            if let Value::Object(e) = entries.remove(0) {
                o.extend(e);
            }
        }
        _ => {
            o.insert("artboards".into(), Value::Array(entries));
        }
    }
    Some(p)
}

/// Run `artboard.setProps` with `common` on each artboard of `indices` that `entry` gives fields for
/// (from its index and rectangle), in one undo step ([`set_props_params`]). Nothing runs when it
/// gives none.
pub(crate) fn set_props(
    app: &mut VectorcraftApp,
    indices: &[usize],
    common: Value,
    entry: impl Fn(usize, Rect) -> Option<Value>,
) -> Result<Value, String> {
    let entries: Vec<Value> = app
        .session
        .active()
        .map(|d| indices.iter().filter_map(|&i| d.doc.artboards.get(i).and_then(|a| entry(i, a.rect))).collect())
        .unwrap_or_default();
    match set_props_params(common, entries) {
        Some(p) => app.run("artboard.setProps", p),
        None => Ok(Value::Null),
    }
}

/// Rename the artboards of `indices` in one undo step (`artboard.rename`): one artboard takes `name`
/// (trimmed), and several take it numbered in panel order, `name-1`, `name-2` and so on.
fn rename(app: &mut VectorcraftApp, indices: &[usize], name: &str) -> Result<Value, String> {
    app.run("artboard.rename", json!({"indices": indices, "name": name}))
}

/// The artboards Delete Artboards deletes (of `n`): the picked ones that aren't locked, without
/// artboard 1 when that would be all of them. Empty when none can go, since one is always left.
pub(crate) fn delete_targets(app: &VectorcraftApp, n: usize) -> Vec<usize> {
    let mut all = deletable(app, n);
    if n > 1 && all.len() == n {
        // Deleting all artboards keeps artboard 1 (the public Delete artboards guide).
        all.retain(|i| *i != 0);
    }
    if n > all.len() { all } else { vec![] }
}

/// Delete the picked artboards that aren't locked and select a remaining one.
pub(crate) fn delete_selected(app: &mut VectorcraftApp) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let all = delete_targets(app, n);
    if !all.is_empty() && app.run("artboard.delete", json!({ "indices": all })).is_ok() {
        select(app, all.first().copied().unwrap_or(0).saturating_sub(1));
    }
}

/// Artboards panel › Delete Empty Artboards: delete the artboards with no artwork, then make the
/// active artboard, where it is now, active again, or else the nearest one before it that is left.
pub(crate) fn delete_empty(app: &mut VectorcraftApp) -> Result<Value, String> {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let active = selected(app, n);
    let r = app.run("artboard.deleteEmpty", json!({}))?;
    let deleted: Vec<usize> = r["deleted"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|i| i as usize).collect();
    // The last artboard up to the active one that is left, numbered as it is now.
    let kept = (0..=active).rev().find(|i| !deleted.contains(i)).unwrap_or(0);
    select(app, kept.saturating_sub(deleted.iter().filter(|d| **d < kept).count()));
    Ok(r)
}

/// What the Artboard tool's canvas menu needs to know about the document's art, which takes a walk
/// over it: [`right_click`] measures it once, and the menu reads it on every frame it is open.
#[derive(Clone, Debug)]
pub struct MenuFacts {
    uid: u64,
    revision: u64,
    picked: Vec<usize>,
    /// The right-click's point, in document coordinates: where the menu's Paste puts artboards.
    pub(crate) at: vectorcraft_geom::Point,
    /// Whether Delete Empty Artboards has an artboard to delete.
    pub(crate) any_empty: bool,
    /// Whether Fit to Artwork Bounds has art to fit a selected artboard to.
    pub(crate) any_art: bool,
}

/// Whether an unlocked artboard of `indices` has art that Fit to Artwork Bounds fits it to, kept by
/// the document ([`vectorcraft_engine::DocState::any_art_to_fit`]): the canvas menu asks on every
/// frame it is open without its [`MenuFacts`].
pub(crate) fn fit_has_art(app: &VectorcraftApp, indices: &[usize]) -> bool {
    app.session.active().is_some_and(|st| st.any_art_to_fit(indices))
}

/// A right-click on the canvas at `p` with the Artboard tool: an artboard under the pointer that
/// isn't selected becomes the only selected one, and the menu's facts are measured.
pub(crate) fn right_click(app: &mut VectorcraftApp, p: vectorcraft_geom::Point) {
    let Some(st) = app.session.active() else { return };
    let (uid, n, hit) = (st.uid, st.doc.artboards.len(), st.doc.artboard_at(p));
    if let Some(i) = hit.filter(|i| !picked(app, n).contains(i)) {
        select(app, i);
    }
    let all = picked(app, n);
    let any_empty = vectorcraft_engine::find_command("artboard.deleteEmpty").is_some_and(|c| (c.enabled)(&app.session).is_ok());
    let any_art = fit_has_art(app, &all);
    let revision = app.session.active().map_or(0, |st| st.revision);
    app.ui.context_artboards = Some(MenuFacts { uid, revision, picked: all, at: p, any_empty, any_art });
}

/// The facts of the open canvas menu, while they still describe the document and the selection.
pub(crate) fn open_menu_facts(app: &VectorcraftApp) -> Option<&MenuFacts> {
    let st = app.session.active()?;
    let n = st.doc.artboards.len();
    app.ui.context_artboards.as_ref().filter(|f| f.uid == st.uid && f.revision == st.revision && f.picked == picked(app, n))
}

/// Run an artboard command chosen from a menu or the command palette while the Artboard tool is in
/// use, with the view and the tool's selection following it as they do in the Artboards panel: a
/// new artboard becomes active, the copies are selected, and a deletion makes a remaining artboard
/// active. None for any other command or tool.
pub(crate) fn run_menu_item(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    if app.session.tool_id() != "artboard" {
        return None;
    }
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    Some(match id {
        "artboard.new" => app.run(id, p.clone()).inspect(|r| {
            if let Some(i) = r["index"].as_u64() {
                select(app, i as usize);
            }
        }),
        "artboard.duplicate" => {
            let all = picked(app, n);
            app.run(id, p.clone()).inspect(|r| select_copies(app, &all, r))
        }
        "artboard.delete" => app.run(id, p.clone()).inspect(|_| {
            let listed = p["indices"].as_array().and_then(|l| l.iter().filter_map(Value::as_u64).min());
            let first = listed.or(p["index"].as_u64()).unwrap_or(0) as usize;
            select(app, first.saturating_sub(1));
        }),
        "artboard.deleteEmpty" => delete_empty(app),
        _ => return None,
    })
}

/// `artboard.selectAll`: the Artboard tool, chosen first when another tool is in use, selects every
/// artboard → the artboards selected, the active one first.
pub(crate) fn select_all_command(app: &mut VectorcraftApp) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    if app.session.tool_id() != "artboard" {
        app.select_tool("artboard");
    }
    select_all(app);
    Ok(json!({ "selected": app.session.tool_options()["selected"] }))
}

/// The rows picked together in the list (Shift-click's range, a Ctrl/Cmd-click's addition), with any tool:
/// the Artboard tool, chosen first, holds the selection, so Properties and the Control bar then show the
/// controls that change every selected artboard. The last of `keys` is the lead and becomes the active
/// artboard; the tool lists the active one first.
fn select_several(app: &mut VectorcraftApp, mut keys: Vec<usize>, n: usize) {
    keys.retain(|k| *k < n);
    let Some(active) = keys.pop() else { return };
    use_artboard_tool(app);
    if let Some(v) = app.view_mut() {
        v.artboard = active;
    }
    keys.insert(0, active);
    app.session.set_tool_option("selected", &json!(keys));
}

/// Choose the Artboard tool when another is in use.
fn use_artboard_tool(app: &mut VectorcraftApp) {
    if app.session.tool_id() != "artboard" {
        app.select_tool("artboard");
    }
}

/// Ctrl/Cmd-click on artboard `i`'s row while the Artboard tool is in use: adds its artboard to the
/// tool's selection and makes it the active one, or takes it out and makes the lowest-numbered
/// remaining artboard active. The only selected artboard stays selected.
pub(crate) fn toggle(app: &mut VectorcraftApp, i: usize, n: usize) {
    let mut all = picked(app, n);
    match all.iter().position(|j| *j == i) {
        Some(at) if all.len() > 1 => {
            all.remove(at);
        }
        Some(_) => return,
        None => all.insert(0, i),
    }
    let active = if all.contains(&i) { i } else { all.first().copied().unwrap_or(0) };
    all.retain(|j| *j != active);
    all.insert(0, active);
    if let Some(v) = app.view_mut() {
        v.artboard = active;
    }
    app.session.set_tool_option("selected", &json!(all));
}

/// Whether Select All's shortcut selects every artboard ([`select_all`]) instead of the art: the list has
/// key focus and the Artboard tool is in use. The list keeps the keys only while its window draws it in every pass, so
/// a closed panel gives them back to the art; the fired commands run after the frame, a pass after the list's last.
pub(crate) fn selects_all(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    app.session.tool_id() == "artboard"
        && app.ui.artboards_ui.focused_pass.is_some_and(|(window, pass)| ctx.cumulative_pass_nr_for(window) <= pass + 1)
}

/// Ctrl/Cmd+A while [`selects_all`]: the Artboard tool selects every artboard, and the active
/// artboard stays active. The art's selection stays as it is.
pub(crate) fn select_all(app: &mut VectorcraftApp) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    if n == 0 {
        return;
    }
    let active = selected(app, n);
    let all: Vec<usize> = std::iter::once(active).chain((0..n).filter(|i| *i != active)).collect();
    app.session.set_tool_option("selected", &json!(all));
}

/// Make artboard `i` the active one, leaving the view where it is: the navigator's and, while it
/// is the tool, the Artboard tool's.
pub(crate) fn select(app: &mut VectorcraftApp, i: usize) {
    if let Some(v) = app.view_mut() {
        v.artboard = i;
    }
    if app.session.tool_id() == "artboard" {
        app.session.set_tool_option("active", &json!(i));
    }
}

/// Add an artboard with the default settings (`artboard.new`) and make it the active artboard.
pub(crate) fn add_new(app: &mut VectorcraftApp) {
    if let Ok(r) = app.run("artboard.new", json!({}))
        && let Some(new) = r["index"].as_u64()
    {
        select(app, new as usize);
    }
}

/// Duplicate artboard `i` (`artboard.duplicate`) and make the copy the active artboard.
fn duplicate(app: &mut VectorcraftApp, i: usize) {
    if let Ok(r) = app.run("artboard.duplicate", json!({ "index": i }))
        && let Some(copy) = r["index"].as_u64()
    {
        select(app, copy as usize);
    }
}

/// Duplicate the selected artboards ([`picked`]) in one undo step (`artboard.duplicate {indices}`)
/// and select the copies, the active artboard's copy active.
pub(crate) fn duplicate_picked(app: &mut VectorcraftApp) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let all = picked(app, n);
    let Ok(r) = app.run("artboard.duplicate", json!({ "indices": all })) else { return };
    select_copies(app, &all, &r);
}

/// After `artboard.duplicate` of the artboards `all` returned `r`: select the copies, the active
/// artboard's copy active.
fn select_copies(app: &mut VectorcraftApp, all: &[usize], r: &Value) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let active = selected(app, n);
    let copies: Vec<usize> =
        r["indices"].as_array().into_iter().flatten().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).collect();
    let Some(first) = all.iter().position(|i| *i == active).and_then(|k| copies.get(k)).or(copies.first()).copied() else { return };
    select(app, first);
    if app.session.tool_id() == "artboard" {
        let rest = copies.iter().copied().filter(|c| *c != first);
        app.session.set_tool_option("selected", &json!(std::iter::once(first).chain(rest).collect::<Vec<_>>()));
    }
}

/// Draw the artboard list and its commands, filling the space above a sized panel's footer.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let rows: Vec<PageListRow> =
        app.session.active().map(|d| d.doc.artboards.iter().map(|a| PageListRow::new(a.name.clone()).locked(a.locked)).collect()).unwrap_or_default();
    if rows.is_empty() {
        super::empty_state(ui, "dc-artboards", tl!("No document"), tl!("Open a document to see its artboards."));
        return;
    }
    let n = rows.len();
    let active = selected(app, n);
    let uid = app.session.active().map_or(0, |d| d.uid);
    let anchor = app.ui.artboards_ui.anchor.filter(|(u, _)| *u == uid).map(|(_, row)| row);
    let mut picked = Selection::new(picked(app, n), Some(active), anchor);
    let mut list = PageList::new(tl!("Artboards"), &rows, &mut picked, active);
    if let Some(row) = app.ui.artboards_ui.rename.take() {
        list = list.start_rename(row);
    }
    let output = list.show(ui);
    // A measuring run of the panel has no focus.
    if !craft::ui::runtime::measure::active(ui.ctx()) {
        app.ui.artboards_ui.focused_pass = output.focused.then(|| (ui.ctx().viewport_id(), ui.ctx().cumulative_pass_nr()));
    }
    if let Some(row) = picked.anchor {
        app.ui.artboards_ui.anchor = Some((uid, row));
    }
    let response = output.response;
    apply(app, output.events);
    if app.ui.artboards_ui.menu.is_some() {
        row_menu(app, ui, &response);
    }
}

/// A row's menu: Rename opens the Rename dialog for the selected artboards ([`picked`]), whose OK
/// runs `artboard.rename` in one undo step, numbering the name when there are several. The menu acts on
/// the selection the list already made on the secondary press.
fn row_menu(app: &mut VectorcraftApp, ui: &mut Ui, list: &egui::Response) {
    let shown = Menu::context(tl!("Artboard"), list).pick(ui, &[Entry::new("rename".to_owned(), tl!("Rename"))]);
    if shown.picked.is_some() {
        crate::menus::invoke(app, "artboard.rename", json!({}));
    }
    if shown.picked.is_some() || shown.closed.is_some() {
        app.ui.artboards_ui.menu = None;
    }
}

/// Run what the list reported, in order. `Activate` has no action: a row has no Enter action here.
pub(crate) fn apply(app: &mut VectorcraftApp, events: Vec<PageListEvent>) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    // A Ctrl/Cmd-click toggles its row and goes to it, and a Shift-click selects a range and goes to its last row: the
    // toggle or the range already chose the active artboard, which a plain go to that row would make the only selected one.
    let chosen: Vec<usize> = events
        .iter()
        .filter_map(|e| match e {
            PageListEvent::Selection(SelectionChange::Toggle(row)) => Some(*row),
            PageListEvent::Selection(SelectionChange::Set(keys)) if keys.len() > 1 => keys.last().copied(),
            _ => None,
        })
        .collect();
    for event in events {
        match event {
            PageListEvent::Go(row) if !chosen.contains(&row) => select(app, row),
            PageListEvent::Selection(SelectionChange::Set(keys)) if keys.len() > 1 => select_several(app, keys, n),
            PageListEvent::Selection(SelectionChange::Set(keys)) => {
                if let Some(&row) = keys.first() {
                    select(app, row);
                }
            }
            PageListEvent::Selection(SelectionChange::Toggle(row)) => {
                use_artboard_tool(app);
                toggle(app, row, n);
            }
            PageListEvent::Rename { row, text, .. } => {
                let text = text.trim();
                if !text.is_empty() {
                    rename(app, &renamed_with(app, row), text).ok();
                }
            }
            PageListEvent::Reorder(edit) => reorder(app, &edit),
            PageListEvent::Lock { row, locked, .. } => {
                app.run("artboard.setProps", json!({"index": row, "locked": locked})).ok();
            }
            PageListEvent::New { duplicate: Some(row), .. } => duplicate(app, row),
            PageListEvent::New { options: true, .. } => {
                app.run("ui.newArtboard", json!({})).ok();
            }
            PageListEvent::New { .. } => add_new(app),
            PageListEvent::Delete(_) => delete_selected(app),
            PageListEvent::Options(row) => {
                select(app, row);
                app.run("tool.options", json!({"tool": "artboard"})).ok();
            }
            PageListEvent::Fit(row) => {
                app.run("view.goToArtboard", json!({"index": row})).ok();
            }
            PageListEvent::Rearrange => open_rearrange(app),
            PageListEvent::ContextMenu(request) => app.ui.artboards_ui.menu = Some(request.key),
            _ => {}
        }
    }
}

/// A row moved by a drag or by Move Up and Move Down: `artboard.reorder` in one undo step, the moved
/// artboard active. Only a finished move counts, and one that stays where it was changes nothing.
fn reorder(app: &mut VectorcraftApp, edit: &EditEvent<craft::ui::contract::ReorderMove<usize>>) {
    let (EditEvent::Commit { after: mv, .. } | EditEvent::Set { after: mv, .. }) = edit else { return };
    if mv.key != mv.to && app.run("artboard.reorder", json!({"index": mv.key, "to": mv.to})).is_ok() {
        select(app, mv.to);
    }
}

/// Open Rearrange All Artboards (#681).
pub(crate) fn open_rearrange(app: &mut VectorcraftApp) {
    app.run("ui.menuDialog", json!({ "command": "artboard.rearrange" })).ok();
}

/// Open Rearrange for the selected artboards ([`picked`]), whose OK waits for a click on the
/// canvas.
pub(crate) fn open_rearrange_selected(app: &mut VectorcraftApp) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    if n > 0 {
        let indices = picked(app, n);
        crate::dialogs::rearrange_artboards::open_selected(app, &indices);
    }
}

/// The rows of the panel's menu.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    let command = |id: &'static str| PanelRow::Command(CommandRef::new(id));
    vec![
        command("ui.artboards.new"),
        command("ui.artboards.duplicate"),
        command("ui.artboards.delete"),
        command("ui.artboards.rename"),
        command("ui.artboards.deleteEmpty"),
        PanelRow::Separator,
        command("artboard.convertToArtboards"),
        PanelRow::Command(CommandRef::with_params("tool.options", json!({"tool": "artboard"}))),
        PanelRow::Command(CommandRef::with_params("ui.menuDialog", json!({"command": "artboard.rearrange"}))),
        command("ui.artboards.rearrange"),
        PanelRow::Separator,
        command("ui.artboards.fitToArt"),
    ]
}

/// The titles and enabled states of [`menu_rows`].
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let mut set = |command: CommandRef, title: &str, enabled: bool| states.set(command, CommandState::new().enabled(enabled).title(title));
    set(CommandRef::new("ui.artboards.new"), tl!("New Artboard"), n > 0);
    set(CommandRef::new("ui.artboards.duplicate"), tl!("Duplicate Artboards"), n > 0);
    set(CommandRef::new("ui.artboards.delete"), tl!("Delete Artboards"), n > 1 && !deletable(app, n).is_empty());
    set(CommandRef::new("ui.artboards.rename"), tl!("Rename"), n > 0);
    set(CommandRef::new("ui.artboards.deleteEmpty"), tl!("Delete Empty Artboards"), crate::menus::enabled(app, "artboard.deleteEmpty"));
    set(CommandRef::new("artboard.convertToArtboards"), tl!("Convert to Artboards"), crate::menus::enabled(app, "artboard.convertToArtboards"));
    set(CommandRef::with_params("tool.options", json!({"tool": "artboard"})), tl!("Artboard Options…"), n > 0);
    set(CommandRef::with_params("ui.menuDialog", json!({"command": "artboard.rearrange"})), tl!("Rearrange All Artboards…"), n > 1);
    set(CommandRef::new("ui.artboards.rearrange"), tl!("Rearrange…"), !tool_selection(app, n).is_empty());
    set(CommandRef::new("ui.artboards.fitToArt"), tl!("Fit to Artwork Bounds"), n > 0);
}

/// Run the panel menu's own row `id` (`ui.artboards.<row>`) → its result; None for any other id. Rename
/// opens the name field on the active artboard's row when that artboard is selected, else on the first
/// selected row. Rearrange… opens Rearrange for the artboards the Artboard tool has selected, and does
/// nothing when it has none.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    match id {
        "ui.artboards.new" => add_new(app),
        "ui.artboards.duplicate" => duplicate_picked(app),
        "ui.artboards.delete" => delete_selected(app),
        "ui.artboards.rename" => {
            let (active, all) = (selected(app, n), picked(app, n));
            if n > 0 {
                app.ui.artboards_ui.rename = Some(if all.contains(&active) { active } else { all.first().copied().unwrap_or(active) });
            }
        }
        "ui.artboards.deleteEmpty" => return Some(delete_empty(app)),
        "ui.artboards.fitToArt" => return Some(app.run("artboard.fitToArt", json!({ "indices": picked(app, n) }))),
        "ui.artboards.rearrange" => {
            if !tool_selection(app, n).is_empty() {
                open_rearrange_selected(app);
            }
        }
        _ => return None,
    }
    Some(Ok(Value::Null))
}

#[cfg(test)]
mod tests;
