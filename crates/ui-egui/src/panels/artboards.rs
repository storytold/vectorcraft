//! Artboards panel: numbered list with inline rename (double-click the name, or the panel menu's
//! Rename; while several artboards are selected, the name typed goes to all of them, numbered),
//! move up / down, new
//! (Alt-click: Artboard Options for the new artboard first), delete; a row dragged onto New
//! Artboard duplicates its artboard (as Duplicate Artboards does, with
//! its art when the Artboard tool's Move/Copy Artwork with Artboard is on). The highlighted row is the active artboard, the one the status bar's navigator shows and
//! Fit Artboard in Window fits: clicking a row makes it active, double-clicking its number also
//! fits it in the window. The options icon at a row's right end opens Artboard Options: a click on
//! the active artboard's icon, or a double-click on another row's icon. The lock column left of it
//! shows a lock on a locked artboard, and a click there locks or unlocks the artboard
//! (`artboard.setProps` with `locked`); Delete leaves locked artboards.

use egui::{Sense, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::ColorMode;
use vectorcraft_geom::Rect;

use super::{alt_held, pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, icons};

/// The Artboard tool's Move Artwork with Artboard and Scale Artwork with Artboard options (#602) as
/// check boxes: the Control bar ([`control_bar`]) and Properties show them while the tool is in use.
pub(crate) fn art_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    art_option_boxes(app, ui, false);
}

/// [`art_options`], or with `compact` the boxes alone, each option's name in its tooltip.
fn art_option_boxes(app: &mut VectorcraftApp, ui: &mut Ui, compact: bool) {
    let opts = app.session.tool_options();
    for (key, label, default) in [("moveArt", tl!("Move Artwork with Artboard"), true), ("scaleArt", tl!("Scale Artwork with Artboard"), false)] {
        let on = opts[key].as_bool().unwrap_or(default);
        let toggled = if compact { widgets::check_tip(ui, "", on, true, label) } else { widgets::check(ui, label, on, true) };
        if toggled {
            app.run("tool.setOption", json!({ "key": key, "value": !on })).ok();
        }
    }
}

/// The room Move and Scale Artwork with Artboard need with their labels in the Control bar; with
/// less, the boxes show alone.
const ART_OPTIONS_WIDTH: f32 = 370.0;

/// The room the Control bar's inline reference point and X/Y/W/H fields for an artboard need; with
/// less, they show only in the Transform link's popover.
const INLINE_FIELDS_WIDTH: f32 = 480.0;

/// The Control bar while the Artboard tool is in use, in place of the object controls: the active
/// artboard's preset and orientation, New Artboard and Delete Artboard, its name, Artboard Options,
/// Rearrange All Artboards and the Transform link (the Transform panel in a popover), which all fit
/// in the narrowest window; then Move and Scale Artwork with Artboard (the boxes alone, with their
/// names in tooltips, when the bar is short of room) and, while the bar has room, the reference
/// point the Transform panel also uses with the artboard's X and Y, and its W and H with the
/// Constrain Width and Height Proportions link. Each control runs the command its Properties panel
/// or Transform panel counterpart runs. Returns false, and draws nothing, while another tool is in
/// use.
pub(crate) fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    if app.session.tool_id() != "artboard" {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let i = selected(app, n);
    let Some(ab) = app.session.active().and_then(|d| d.doc.artboards.get(i)).cloned() else { return true };
    let r = ab.rect;
    super::properties::artboard_size_row(app, ui, i, r.width(), r.height());
    if widgets::icon_button(ui, "dc-new-item", tl!("New Artboard"), false, 24.0).clicked() && app.run("artboard.new", json!({})).is_ok() {
        select(app, n);
    }
    if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Artboard"), false, n > 1, 24.0).clicked() {
        delete_selected(app);
    }
    ui.separator();
    // The field shows the name as it is (names are never translated). A new name is applied on Enter
    // or a click elsewhere, and an empty field or Escape keeps the old one, as in the Properties panel.
    ui.label(egui::RichText::new(tl!("Name:")).size(12.0).color(t.text));
    if let Some(name) = widgets::text_field_named(ui, ("cb-ab", "name"), Some(tl!("Name:")), Some(&ab.name), 120.0, 1).filter(|n| !n.is_empty()) {
        app.run("artboard.setProps", json!({"index": i, "name": name})).ok();
    }
    if widgets::icon_button(ui, "dc-artboard-options", tl!("Artboard Options"), false, 24.0).clicked() {
        app.run("tool.options", json!({"tool": "artboard"})).ok();
    }
    if widgets::icon_button_enabled(ui, "dc-rearrange", tl!("Rearrange All Artboards"), false, n > 1, 24.0).clicked() {
        open_rearrange(app);
    }
    ui.separator();
    super::transform::link(app, ui);
    ui.separator();
    art_option_boxes(app, ui, ui.available_width() < ART_OPTIONS_WIDTH);
    if ui.available_width() < INLINE_FIELDS_WIDTH {
        return true;
    }
    ui.separator();
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    let refi = super::transform::reference_index(ui);
    super::transform::reference_locator(ui, refi);
    let link = super::transform::proportional(app, ui);
    let rp = vectorcraft_geom::reference_point(r, refi);
    for (k, lbl, v) in [("x", "X:", rp.x - origin.x), ("y", "Y:", rp.y - origin.y), ("width", "W:", r.width()), ("height", "H:", r.height())] {
        // The W/H link sits between W and H.
        if k == "height" {
            super::transform::constrain_link(app, ui);
        }
        widgets::field_label(ui, egui::RichText::new(tl!(lbl)).size(12.0).color(t.text_dim));
        let set = if matches!(k, "width" | "height") {
            widgets::size_field(ui, ("cb-ab", k), Some(v), units, 80.0)
        } else {
            widgets::num_field(ui, ("cb-ab", k), Some(v), units, 80.0)
        };
        if let Some(nv) = set {
            let xywh = match k {
                "x" => [Some(nv + origin.x), None, None, None],
                "y" => [None, Some(nv + origin.y), None, None],
                "width" => [None, None, Some(nv), None],
                _ => [None, None, None, Some(nv)],
            };
            super::transform::set_artboard(app, i, refi, xywh, link);
        }
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
    let tool = (app.session.tool_id() == "artboard").then(|| app.session.tool_options()["selected"].clone());
    let mut all: Vec<usize> = tool
        .iter()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|i| usize::try_from(i).ok())
        .filter(|i| *i < n)
        .collect();
    if all.is_empty() {
        all.push(selected(app, n));
    }
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

/// Rename the artboards of `indices` in one undo step (`artboard.setProps`): one artboard takes
/// `name` (trimmed), and several take it numbered in panel order, `name-1`, `name-2` and so on.
fn rename(app: &mut VectorcraftApp, indices: &[usize], name: &str) -> Result<Value, String> {
    let name = name.trim();
    let mut all = indices.to_vec();
    all.sort_unstable();
    all.dedup();
    let several = all.len() > 1;
    let numbered = |k: usize| if several { format!("{name}-{}", k + 1) } else { name.to_string() };
    let entries = all.iter().enumerate().map(|(k, i)| json!({"index": i, "name": numbered(k)})).collect();
    match set_props_params(json!({}), entries) {
        Some(p) => app.run("artboard.setProps", p),
        None => Ok(Value::Null),
    }
}

/// Open the name field on artboard `i`'s row. The name typed there goes to the artboards
/// [`renamed_with`] gives for `i` now, numbered when there are several ([`rename`]).
fn start_rename(ctx: &egui::Context, app: &VectorcraftApp, i: usize) {
    set_pstate(ctx, "ab-edit", Some(i));
    set_pstate(ctx, "ab-edit-all", renamed_with(app, i));
}

/// Delete the picked artboards that aren't locked and select a remaining one.
pub(crate) fn delete_selected(app: &mut VectorcraftApp) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let mut all = deletable(app, n);
    if all.is_empty() {
        return;
    }
    if n > 1 && all.len() == n {
        // Deleting all artboards keeps artboard 1 (the public Delete artboards guide).
        all.retain(|i| *i != 0);
    }
    if n > all.len() && app.run("artboard.delete", json!({ "indices": all })).is_ok() {
        select(app, all.first().copied().unwrap_or(0).saturating_sub(1));
    }
}

/// Artboards panel › Delete Empty Artboards: delete the artboards with no artwork, then make the
/// active artboard, where it is now, active again, or else the nearest one before it that is left.
pub(crate) fn delete_empty(app: &mut VectorcraftApp) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let active = selected(app, n);
    let Ok(r) = app.run("artboard.deleteEmpty", json!({})) else { return };
    let deleted: Vec<usize> = r["deleted"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|i| i as usize).collect();
    // The last artboard up to the active one that is left, numbered as it is now.
    let kept = (0..=active).rev().find(|i| !deleted.contains(i)).unwrap_or(0);
    select(app, kept.saturating_sub(deleted.iter().filter(|d| **d < kept).count()));
}

/// Shift-click on artboard `i`'s row while the Artboard tool is in use: the tool selects the
/// artboards from `anchor` to `i`, and `i` becomes the active one. `anchor` is the last row clicked
/// without Shift or where the last range started; when its artboard is no longer selected, the
/// range starts from the active artboard. Returns where the range starts, the anchor for the next
/// Shift-click.
fn select_range(app: &mut VectorcraftApp, anchor: Option<usize>, i: usize, n: usize) -> usize {
    let from = anchor.filter(|a| picked(app, n).contains(a)).unwrap_or_else(|| selected(app, n));
    let mut all: Vec<usize> = (from.min(i)..=from.max(i)).filter(|j| *j != i).collect();
    all.insert(0, i);
    if let Some(v) = app.view_mut() {
        v.artboard = i;
    }
    app.session.set_tool_option("selected", &json!(all));
    from
}

/// Ctrl/Cmd-click on artboard `i`'s row while the Artboard tool is in use: adds its artboard to the
/// tool's selection and makes it the active one, or takes it out and makes the lowest-numbered
/// remaining artboard active. The only selected artboard stays selected.
fn toggle(app: &mut VectorcraftApp, i: usize, n: usize) {
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

/// Whether the panel has the keyboard: one of its rows was clicked, and in every frame since then
/// the panel was drawn and the pointer was not pressed outside its list.
fn has_keyboard(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    pstate::<Option<u64>>(ctx, "ab-keys").is_some_and(|f| f + 1 >= app.frame_iteration())
}

/// Whether Select All's shortcut selects every artboard ([`select_all`]) instead of the art: the
/// panel has the keyboard and the Artboard tool is in use. [`crate::shortcuts::handle`] and
/// [`crate::menus::invoke_from_system_menu`] read it before they run `select.all`, which they reach
/// only while no text field has the keyboard.
pub(crate) fn selects_all(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    app.session.tool_id() == "artboard" && has_keyboard(app, ctx)
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

/// A row dragged in the list: the artboard's index.
#[derive(Clone, Copy)]
struct RowDrag(usize);

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
    let active = selected(app, n);
    let Ok(r) = app.run("artboard.duplicate", json!({ "indices": all })) else { return };
    let copies: Vec<usize> =
        r["indices"].as_array().into_iter().flatten().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).collect();
    let Some(first) = all.iter().position(|i| *i == active).and_then(|k| copies.get(k)).or(copies.first()).copied() else { return };
    select(app, first);
    if app.session.tool_id() == "artboard" {
        let rest = copies.iter().copied().filter(|c| *c != first);
        app.session.set_tool_option("selected", &json!(std::iter::once(first).chain(rest).collect::<Vec<_>>()));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let abs: Vec<String> = app.session.active().map(|d| d.doc.artboards.iter().map(|a| a.name.clone()).collect()).unwrap_or_default();
    let locked: Vec<bool> = app.session.active().map(|d| d.doc.artboards.iter().map(|a| a.locked).collect()).unwrap_or_default();
    if abs.is_empty() {
        super::empty_state(ui, "dc-artboards", tl!("No document"), tl!("Open a document to see its artboards."));
        return;
    }
    let sel = selected(app, abs.len());
    let all = picked(app, abs.len());
    let editing: Option<usize> = pstate(ui.ctx(), "ab-edit");
    let mut row_clicked = false;
    let list_rect = widgets::list_box(ui, |ui| {
        ui.set_min_height(110.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        let scrolled = egui::ScrollArea::vertical().id_salt("ab-scroll").max_height(220.0).show(ui, |ui| {
            for (i, name) in abs.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click_and_drag());
                if resp.drag_started() {
                    egui::DragAndDrop::set_payload(ui.ctx(), RowDrag(i));
                }
                if all.contains(&i) {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                ui.painter().text(
                    pos2(r.left() + 22.0, r.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{}", i + 1),
                    egui::FontId::proportional(12.0),
                    t.text_dim,
                );
                let name_rect = egui::Rect::from_min_max(pos2(r.left() + 32.0, r.top() + 2.0), pos2(r.right() - 46.0, r.bottom() - 2.0));
                if editing == Some(i) {
                    // The artboards the field renames: the artboards selected when it opened, when
                    // there were several and this row's artboard is one of them, else this row's
                    // artboard alone. With several, the field starts empty.
                    let opened_on: Vec<usize> = pstate(ui.ctx(), "ab-edit-all");
                    let targets = if opened_on.len() > 1 && opened_on.contains(&i) { opened_on } else { vec![i] };
                    let shown = if targets.len() > 1 { "" } else { name.as_str() };
                    let id = ui.scope_id().with(("ab-name", i));
                    let mut buf: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.to_string());
                    let kept = widgets::own_field_shift_enter(ui, id, &mut buf, shown);
                    let te = ui.put(name_rect, egui::TextEdit::singleline(&mut buf).id(id).font(egui::FontId::proportional(12.0)));
                    if !te.has_focus() && !te.lost_focus() {
                        te.request_focus();
                    }
                    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
                    let renames = !buf.trim().is_empty() && buf != shown;
                    if te.lost_focus() {
                        set_pstate::<Option<usize>>(ui.ctx(), "ab-edit", None);
                        set_pstate::<Vec<usize>>(ui.ctx(), "ab-edit-all", vec![]);
                        ui.data_mut(|d| d.remove::<String>(id));
                        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && renames {
                            rename(app, &targets, &buf).ok();
                        }
                    } else if kept && renames {
                        // Shift+Enter renames them and keeps the field open.
                        rename(app, &targets, &buf).ok();
                    }
                } else {
                    ui.painter().text(name_rect.left_center(), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.5), t.text);
                }
                // The lock column, right of the name: the lock shows while the artboard is locked
                // (dimmed under the pointer while it isn't), and a click locks or unlocks it.
                let is_locked = locked.get(i).copied().unwrap_or(false);
                let lock = egui::Rect::from_center_size(r.right_center() - vec2(36.0, 0.0), vec2(14.0, 14.0));
                let lresp = ui.interact(lock, ui.scope_id().with(("ab-lock", i)), Sense::click());
                if is_locked {
                    icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.icon);
                } else if lresp.hovered() {
                    icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.text_disabled);
                }
                if lresp.clicked() {
                    app.run("artboard.setProps", json!({"index": i, "locked": !is_locked})).ok();
                }
                let opt = egui::Rect::from_center_size(r.right_center() - vec2(14.0, 0.0), vec2(14.0, 14.0));
                let oresp = ui.interact(opt, ui.scope_id().with(("ab-opt", i)), Sense::click());
                icons::paint(ui, "dc-artboard-options", opt, if oresp.hovered() { t.text_strong } else { t.icon });
                let oresp = oresp.on_hover_text(tl!("Artboard Options"));
                // A click on the active artboard's icon opens its Artboard Options. A click on
                // another row's icon makes that artboard active, so a double-click opens its options.
                if oresp.clicked() {
                    select(app, i);
                    if i == sel || oresp.double_clicked() {
                        app.run("tool.options", json!({ "tool": "artboard" })).ok();
                    }
                }
                let m = ui.input(|inp| inp.modifiers);
                let tool = app.session.tool_id() == "artboard";
                row_clicked |= resp.clicked();
                if resp.clicked() && tool && m.shift {
                    let from = select_range(app, pstate(ui.ctx(), "ab-anchor"), i, abs.len());
                    set_pstate(ui.ctx(), "ab-anchor", Some(from));
                } else if resp.clicked() {
                    set_pstate(ui.ctx(), "ab-anchor", Some(i));
                    // The second click of a Ctrl/Cmd double-click leaves the row's artboard
                    // selected. On the name, the double-click then opens the field for the
                    // selection with it.
                    if tool && m.command {
                        if !(resp.double_clicked() && picked(app, abs.len()).contains(&i)) {
                            toggle(app, i, abs.len());
                        }
                    } else {
                        select(app, i);
                    }
                }
                // Double-clicking the number goes to the artboard (fits it in the window); the name
                // renames it.
                let on_number = resp.interact_pointer_pos().is_some_and(|p| p.x < name_rect.left());
                if resp.double_clicked() && on_number {
                    app.run("view.goToArtboard", json!({ "index": i })).ok();
                } else if resp.double_clicked() {
                    start_rename(ui.ctx(), app, i);
                }
            }
        });
        scrolled.inner_rect
    });
    // Keyboard: the panel takes it with a click on a row and gives it up when the pointer is
    // pressed outside the list or the panel is not drawn (`ab-keys`: the frame it last had it in).
    let pressed_outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !list_rect.contains(p)));
    let keys = row_clicked || (has_keyboard(app, ui.ctx()) && !pressed_outside);
    set_pstate(ui.ctx(), "ab-keys", keys.then_some(app.frame_iteration()));
    let n = abs.len();
    widgets::bottom_bar(ui, |ui| {
        if widgets::icon_button_enabled(ui, "dc-rearrange", tl!("Rearrange All Artboards"), false, n > 1, 24.0).clicked() {
            open_rearrange(app);
        }
        ui.add_space((ui.available_width() - 4.0 * 28.0).max(0.0));
        if widgets::icon_button_enabled(ui, "dc-arrow-up", tl!("Move Up"), false, sel > 0, 24.0).clicked()
            && app.run("artboard.reorder", json!({"index": sel, "to": sel - 1})).is_ok()
        {
            select(app, sel - 1);
        }
        if widgets::icon_button_enabled(ui, "dc-arrow-down", tl!("Move Down"), false, sel + 1 < n, 24.0).clicked()
            && app.run("artboard.reorder", json!({"index": sel, "to": sel + 1})).is_ok()
        {
            select(app, sel + 1);
        }
        let new = widgets::icon_button(ui, "dc-new-item", tl!("New Artboard"), false, 24.0);
        if new.dnd_hover_payload::<RowDrag>().is_some() {
            ui.painter().rect_stroke(new.rect, 3.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
        }
        if let Some(row) = new.dnd_release_payload::<RowDrag>() {
            duplicate(app, row.0);
        } else if new.clicked() && alt_held(ui) {
            // Alt-click: Artboard Options for the new artboard, which OK adds.
            app.run("ui.newArtboard", json!({})).ok();
        } else if new.clicked() && app.run("artboard.new", json!({})).is_ok() {
            select(app, n);
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Artboard"), false, n > 1 && !deletable(app, n).is_empty(), 24.0).clicked() {
            delete_selected(app);
        }
    });
}

/// Open Rearrange All Artboards (#681).
pub(crate) fn open_rearrange(app: &mut VectorcraftApp) {
    app.run("ui.menuDialog", json!({ "command": "artboard.rearrange" })).ok();
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = app.session.active().map(|d| d.doc.artboards.len()).unwrap_or(0);
    let sel = selected(app, n);
    if menu_item(ui, tl!("New Artboard"), n > 0, false) {
        app.run("artboard.new", json!({})).ok();
    }
    if menu_item(ui, tl!("Duplicate Artboards"), n > 0, false) {
        duplicate_picked(app);
    }
    if menu_item(ui, tl!("Delete Artboards"), n > 1 && !deletable(app, n).is_empty(), false) {
        delete_selected(app);
    }
    if menu_item(ui, tl!("Rename"), n > 0, false) {
        // When the active artboard is selected, the field opens on its row; otherwise it opens on the
        // first selected row.
        let all = picked(app, n);
        start_rename(ui.ctx(), app, if all.contains(&sel) { sel } else { all.first().copied().unwrap_or(sel) });
    }
    if menu_item(ui, tl!("Delete Empty Artboards"), crate::menus::enabled(app, "artboard.deleteEmpty"), false) {
        delete_empty(app);
    }
    ui.separator();
    if menu_item(ui, tl!("Convert to Artboards"), crate::menus::enabled(app, "artboard.convertToArtboards"), false) {
        crate::menus::invoke(app, "artboard.convertToArtboards", json!({}));
    }
    if menu_item(ui, tl!("Artboard Options…"), n > 0, false) {
        app.run("tool.options", json!({ "tool": "artboard" })).ok();
    }
    if menu_item(ui, tl!("Rearrange All Artboards…"), n > 1, false) {
        open_rearrange(app);
    }
    ui.separator();
    if menu_item(ui, tl!("Fit to Artwork Bounds"), n > 0, false) {
        app.run("artboard.fitToArt", json!({ "indices": picked(app, n) })).ok();
    }
}

#[cfg(test)]
mod tests;
