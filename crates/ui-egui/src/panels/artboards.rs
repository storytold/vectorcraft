//! Artboards panel: numbered list with inline rename (double-click the name), move up / down, new
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
    if let Some(name) = widgets::text_field(ui, ("cb-ab", "name"), Some(&ab.name), 120.0, 1).filter(|n| !n.is_empty()) {
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
            super::transform::set_artboard(app, i, r, refi, xywh, link);
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
    widgets::list_box(ui, |ui| {
        ui.set_min_height(110.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt("ab-scroll").max_height(220.0).show(ui, |ui| {
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
                    let id = ui.id().with(("ab-name", i));
                    let mut buf: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| name.clone());
                    let te = ui.put(name_rect, egui::TextEdit::singleline(&mut buf).id(id).font(egui::FontId::proportional(12.0)));
                    if !te.has_focus() && !te.lost_focus() {
                        te.request_focus();
                    }
                    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
                    if te.lost_focus() {
                        set_pstate::<Option<usize>>(ui.ctx(), "ab-edit", None);
                        ui.data_mut(|d| d.remove::<String>(id));
                        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && !buf.trim().is_empty() && buf != *name {
                            app.run("artboard.setProps", json!({"index": i, "name": buf.trim()})).ok();
                        }
                    }
                } else {
                    ui.painter().text(name_rect.left_center(), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.5), t.text);
                }
                // The lock column, right of the name: the lock shows while the artboard is locked
                // (dimmed under the pointer while it isn't), and a click locks or unlocks it.
                let is_locked = locked.get(i).copied().unwrap_or(false);
                let lock = egui::Rect::from_center_size(r.right_center() - vec2(36.0, 0.0), vec2(14.0, 14.0));
                let lresp = ui.interact(lock, ui.id().with(("ab-lock", i)), Sense::click());
                if is_locked {
                    icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.icon);
                } else if lresp.hovered() {
                    icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.text_disabled);
                }
                if lresp.clicked() {
                    app.run("artboard.setProps", json!({"index": i, "locked": !is_locked})).ok();
                }
                let opt = egui::Rect::from_center_size(r.right_center() - vec2(14.0, 0.0), vec2(14.0, 14.0));
                let oresp = ui.interact(opt, ui.id().with(("ab-opt", i)), Sense::click());
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
                if resp.clicked() && tool && m.shift {
                    let from = select_range(app, pstate(ui.ctx(), "ab-anchor"), i, abs.len());
                    set_pstate(ui.ctx(), "ab-anchor", Some(from));
                } else if resp.clicked() {
                    set_pstate(ui.ctx(), "ab-anchor", Some(i));
                    if tool && m.command {
                        toggle(app, i, abs.len());
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
                    set_pstate(ui.ctx(), "ab-edit", Some(i));
                }
            }
        });
    });
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
        set_pstate(ui.ctx(), "ab-edit", Some(sel));
    }
    menu_item(ui, tl!("Delete Empty Artboards"), false, false);
    ui.separator();
    menu_item(ui, tl!("Convert to Artboards"), false, false);
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
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// Clicking a row makes its artboard the active one (the navigator's, Fit Artboard in Window's).
    #[test]
    fn clicking_a_row_makes_its_artboard_active() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 200, "height": 150})).unwrap();
        app.view_mut().unwrap().artboard = 0;
        let ctx = egui::Context::default();
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        // Where the second row's name is painted.
        let at = frame(&mut app, vec![])
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == "Artboard 2" => Some(t.pos + vec2(4.0, 4.0)),
                _ => None,
            })
            .unwrap();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for e in [egui::Event::PointerMoved(at), button(true), button(false)] {
            frame(&mut app, vec![e]);
        }
        assert_eq!(app.view().unwrap().artboard, 1, "Artboard 2 is active");
        // The navigator moves it on; the panel follows.
        crate::menus::invoke(&mut app, "view.goToArtboard", json!({"index": "previous"}));
        assert_eq!(selected(&app, 2), 0, "and the panel follows the navigator");
    }

    /// #1006: Select › All on Active Artboard selects the art of the artboard made active here,
    /// and an explicit artboard still wins.
    #[test]
    fn all_on_active_artboard_takes_the_active_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 2})).unwrap();
        let mut rect = |x| app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        let (a, b) = (rect(10), rect(130));
        let picked = |app: &mut VectorcraftApp, params| {
            crate::menus::invoke(app, "select.allOnArtboard", params);
            let ids: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|n| n.0).collect();
            json!(ids)
        };
        select(&mut app, 1);
        assert_eq!(picked(&mut app, json!({})), json!([b]));
        select(&mut app, 0);
        assert_eq!(picked(&mut app, json!({})), json!([a]));
        assert_eq!(picked(&mut app, json!({"artboard": 1})), json!([b]));
    }

    /// Object › Artboards › Fit to Artwork Bounds and Fit to Selected Art resize the artboard made
    /// active here, as the panel menu's Fit to Artwork Bounds does, and leave artboard 1 as it was;
    /// an explicit index still wins.
    #[test]
    fn fit_to_artwork_bounds_and_selected_art_resize_the_active_artboard() {
        for id in ["artboard.fitToArt", "artboard.fitToSelection"] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 2})).unwrap();
            // Partly on artboard 1 and partly on artboard 2.
            app.session.execute("shape.rectangle", &json!({"x": 90, "y": 10, "width": 60, "height": 30})).unwrap();
            app.session.execute("paint.setStroke", &json!({"none": true})).unwrap();
            let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
            let (first, art) = (boards(&app)[0], vectorcraft_geom::Rect::new(90.0, 10.0, 150.0, 40.0));
            select(&mut app, 1);
            crate::menus::invoke(&mut app, id, json!({}));
            assert_eq!(boards(&app), [first, art], "{id} resizes Artboard 2, the active one");
            crate::menus::invoke(&mut app, id, json!({"index": 0}));
            assert_eq!(boards(&app), [art, art], "{id} with an index resizes that artboard");
        }
    }

    /// With artboards 1 and 3 selected with the Artboard tool, Object › Artboards › Fit to Artwork
    /// Bounds fits each of them to the art on it and leaves artboard 2 as it was.
    #[test]
    fn fit_to_artwork_bounds_fits_each_selected_artboard_to_its_own_art() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
        for (x, h) in [(10, 30), (130, 20), (250, 40)] {
            app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": h})).unwrap();
        }
        app.session.execute("select.all", &json!({})).unwrap();
        app.session.execute("paint.setStroke", &json!({"none": true})).unwrap();
        let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
        let was = boards(&app);
        app.select_tool("artboard");
        select(&mut app, 0);
        app.session.set_tool_option("selected", &json!([0, 2]));
        crate::menus::invoke(&mut app, "artboard.fitToArt", json!({}));
        let rect = vectorcraft_geom::Rect::new;
        assert_eq!(boards(&app), [rect(10.0, 10.0, 30.0, 40.0), was[1], rect(250.0, 10.0, 270.0, 50.0)]);
    }

    /// With the Artboard tool, Shift-click on a row selects the artboards from the last row clicked
    /// without Shift to it and makes it the active one, Ctrl/Cmd-click adds its artboard to the
    /// tool's selection or takes it out (#1139), a plain click selects it alone, and every selected
    /// row is highlighted.
    #[test]
    fn shift_clicking_rows_selects_several_artboards() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        for x in [400, 800, 1200] {
            app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
        }
        app.select_tool("artboard");
        select(&mut app, 1);
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let raw = egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        let shapes = frame(&mut app, 0.0, vec![]);
        // `row(k)` returns the point where row `k`'s name is painted.
        let row = |k: usize| {
            shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Text(t) if t.galley.text() == format!("Artboard {k}") => Some(t.pos + vec2(4.0, 4.0)),
                    _ => None,
                })
                .unwrap()
        };
        let click = |k: usize, modifiers: egui::Modifiers| {
            let at = row(k);
            let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
            vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(at), b(true), b(false)]
        };
        let highlighted = |shapes: Vec<egui::epaint::ClippedShape>| {
            shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).row_selected)).count()
        };
        // With no row clicked yet, Shift-click ranges from the active artboard.
        frame(&mut app, 1.0, click(4, egui::Modifiers::SHIFT));
        assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![1, 2, 3], 3));
        frame(&mut app, 1.5, click(3, egui::Modifiers::SHIFT));
        assert_eq!(picked(&app, 4), [1, 2], "the next Shift-click ranges from the same artboard");
        frame(&mut app, 2.0, click(1, egui::Modifiers::NONE));
        assert_eq!(picked(&app, 4), [0]);
        frame(&mut app, 3.0, click(3, egui::Modifiers::SHIFT));
        assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![0, 1, 2], 2), "Artboards 1 to 3, Artboard 3 active");
        assert_eq!(highlighted(frame(&mut app, 4.0, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)])), 3, "three rows highlighted");
        // A second Shift-click ranges from the same row.
        frame(&mut app, 5.0, click(2, egui::Modifiers::SHIFT));
        assert_eq!(picked(&app, 4), [0, 1]);
        frame(&mut app, 6.0, click(3, egui::Modifiers::SHIFT));
        frame(&mut app, 7.0, click(2, egui::Modifiers::COMMAND));
        assert_eq!(picked(&app, 4), [0, 2], "Ctrl/Cmd-click takes Artboard 2 out");
        frame(&mut app, 8.0, click(2, egui::Modifiers::COMMAND));
        assert_eq!(picked(&app, 4), [0, 1, 2], "and puts it back");
        frame(&mut app, 9.0, click(4, egui::Modifiers::NONE));
        assert_eq!(picked(&app, 4), [3]);
        assert_eq!(highlighted(frame(&mut app, 10.0, vec![])), 1);
        // With another tool, a Shift-click makes the row's artboard active, as a plain click does.
        app.select_tool("selection");
        frame(&mut app, 11.0, click(2, egui::Modifiers::SHIFT));
        assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![1], 1));
    }

    /// Double-clicking a row's number goes to its artboard and its name renames it; a click on
    /// another row's options icon makes that artboard the active one, and the Artboard tool's
    /// artboard is the active one.
    #[test]
    fn double_clicks_go_to_or_rename_and_options_edits_the_rows_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 200, "height": 150})).unwrap();
        app.view_mut().unwrap().artboard = 0;
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        let shapes = frame(&mut app, 0.0, vec![]);
        let name = shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == "Artboard 2" => Some(t.pos + vec2(4.0, 4.0)),
                _ => None,
            })
            .unwrap();
        // The highlighted row (Artboard 1's), for its Options button at the right end.
        let row = shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).row_selected => Some(r.rect),
                _ => None,
            })
            .unwrap();
        let events = |at: egui::Pos2, clicks: usize| {
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            std::iter::once(egui::Event::PointerMoved(at)).chain((0..clicks).flat_map(|_| [button(true), button(false)])).collect::<Vec<_>>()
        };
        // Artboard 2's number: it is active, no rename starts.
        frame(&mut app, 1.0, events(name - vec2(16.0, 0.0), 2));
        assert_eq!(app.view().unwrap().artboard, 1);
        assert_eq!(pstate::<Option<usize>>(&ctx, "ab-edit"), None);
        // Its name: a rename.
        frame(&mut app, 2.0, events(name, 2));
        assert_eq!(pstate::<Option<usize>>(&ctx, "ab-edit"), Some(1));
        set_pstate::<Option<usize>>(&ctx, "ab-edit", None);
        frame(&mut app, 3.0, vec![]);
        // A click on Artboard 1's options icon makes Artboard 1 active, and the Artboard tool,
        // chosen next, starts on it.
        frame(&mut app, 4.0, events(egui::pos2(row.right() - 14.0, row.center().y), 1));
        app.select_tool("artboard");
        assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["active"].clone()), (0, json!(0)));
        // While the Artboard tool is chosen, clicking a row moves the tool to that artboard too.
        frame(&mut app, 5.0, events(name, 1));
        assert_eq!(app.session.tool_options()["active"], json!(1));
        // And clicking an artboard with the tool makes it the active one, highlighted in the panel.
        let view = app.view_info();
        for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
            crate::canvas::dispatch(&mut app, &vectorcraft_tools::PointerEvent::new(kind, 100.0, 100.0), view);
        }
        assert_eq!((app.session.tool_options()["active"].clone(), selected(&app, 2)), (json!(0), 0));
    }

    /// A row's options icon opens Artboard Options for that row's artboard and leaves the tool as
    /// it is (#1876): a click on the active artboard's icon, or a double-click on another row's
    /// icon, whose first click only makes that artboard active. With the Artboard tool in use, the
    /// tool moves to that artboard too.
    #[test]
    fn the_options_icon_opens_artboard_options_for_its_rows_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 100, "height": 50})).unwrap();
        app.view_mut().unwrap().artboard = 0;
        let tool = app.session.tool_id().to_string();
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        // The highlighted row (Artboard 1's). The rows are 24 points apart, and each icon's
        // center is 14 points in from the row's right end.
        let row = frame(&mut app, 0.0, vec![])
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).row_selected => Some(r.rect),
                _ => None,
            })
            .unwrap();
        let icon = |k: f32| egui::pos2(row.right() - 14.0, row.center().y + 24.0 * k);
        let events = |at: egui::Pos2, clicks: usize| {
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            std::iter::once(egui::Event::PointerMoved(at)).chain((0..clicks).flat_map(|_| [button(true), button(false)])).collect::<Vec<_>>()
        };
        let opened = |app: &mut VectorcraftApp| app.ui.dialog.take().map(|d| (d.kind.clone(), d.f64("index", 9.0), d.f64("width", 0.0)));
        // Artboard 2's icon: the first click makes it active, the second (a double-click) opens its
        // options; no rename starts.
        frame(&mut app, 1.0, events(icon(1.0), 1));
        assert_eq!((app.view().unwrap().artboard, opened(&mut app)), (1, None));
        frame(&mut app, 1.1, events(icon(1.0), 1));
        assert_eq!(opened(&mut app), Some(("artboardOptions".to_string(), 1.0, 100.0)));
        assert_eq!((app.session.tool_id(), pstate::<Option<usize>>(&ctx, "ab-edit")), (tool.as_str(), None));
        // The active artboard's icon: one click opens its options.
        frame(&mut app, 3.0, events(icon(1.0), 1));
        assert_eq!(opened(&mut app), Some(("artboardOptions".to_string(), 1.0, 100.0)));
        // With the Artboard tool, a double-click on Artboard 1's icon (both clicks in one frame)
        // opens Artboard 1's options, and the tool moves to it.
        app.select_tool("artboard");
        frame(&mut app, 5.0, events(icon(0.0), 2));
        assert_eq!(opened(&mut app), Some(("artboardOptions".to_string(), 0.0, 200.0)));
        assert_eq!((app.session.tool_id(), app.session.tool_options()["active"].clone()), ("artboard", json!(0)));
    }

    /// The panel menu's Artboard Options… opens Artboard Options for the active artboard and leaves
    /// the tool as it is.
    #[test]
    fn the_panel_menus_artboard_options_opens_the_active_artboards_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 100, "height": 50})).unwrap();
        select(&mut app, 1);
        let tool = app.session.tool_id().to_string();
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: screen, ..Default::default() }, |ui| menu(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        let item = frame(&mut app, vec![])
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text().trim_start() == "Artboard Options…" => Some(t.visual_bounding_rect().center()),
                _ => None,
            })
            .unwrap();
        let button = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(item), button(true), button(false)]);
        let d = app.ui.dialog.take().unwrap();
        assert_eq!((d.kind.as_str(), d.f64("index", 9.0), d.f64("x", 0.0)), ("artboardOptions", 1.0, 400.0));
        assert_eq!(app.session.tool_id(), tool);
    }

    /// With two artboards selected with the Artboard tool, the panel menu's Duplicate Artboards
    /// copies both, with their art, in one undo step, and the tool then selects the copies, the
    /// active artboard's copy active.
    #[test]
    fn the_panel_menus_duplicate_artboards_copies_every_selected_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        for x in [400, 800] {
            app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
        }
        for x in [10, 410] {
            app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
        }
        app.select_tool("artboard");
        select(&mut app, 1);
        app.session.set_tool_option("selected", &json!([1, 0]));
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: screen, ..Default::default() }, |ui| menu(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        let item = frame(&mut app, vec![])
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text().trim_start() == "Duplicate Artboards" => Some(t.visual_bounding_rect().center()),
                _ => None,
            })
            .unwrap();
        let button = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(item), button(true), button(false)]);
        let d = &app.session.active().unwrap().doc;
        let names: Vec<&str> = d.artboards.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Artboard 1", "Artboard 2", "Artboard 3", "Artboard 1 copy", "Artboard 2 copy"]);
        assert_eq!(d.layers[0].children().unwrap().len(), 4, "the squares and their copies");
        assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (4, json!([4, 3])), "Artboard 2's copy is active");
        app.run("edit.undo", json!({})).unwrap();
        let d = &app.session.active().unwrap().doc;
        assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (3, 2), "one undo step");
    }

    /// One headless frame of the Control bar, 2,000 points wide, with `events` → the texts painted,
    /// with their rects.
    fn bar_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
        bar_frame_in(app, ctx, events, 2000.0)
    }

    /// [`bar_frame`] in a window `width` points wide.
    fn bar_frame_in(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, width: f32) -> Vec<(String, egui::Rect)> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 300.0));
        let mut out =
            ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| crate::chrome::control_bar(app, ui));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
                _ => None,
            })
            .collect()
    }

    /// A context with the app's fonts, which the Control bar's label uses.
    fn bar_context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    }

    /// Two frames of the Control bar → the texts the second one painted.
    fn bar_texts(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(String, egui::Rect)> {
        bar_frame(app, ctx, vec![]);
        bar_frame(app, ctx, vec![])
    }

    /// Where `label` is painted in `texts`.
    fn text_rect(texts: &[(String, egui::Rect)], label: &str) -> egui::Rect {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| *r).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"))
    }

    /// A press and a release at `at` on the Control bar, a frame each.
    fn bar_click(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        bar_frame(app, ctx, vec![egui::Event::PointerMoved(at), button(true)]);
        bar_frame(app, ctx, vec![button(false)]);
    }

    /// Click the Control bar's field at `at`, select all of it, type `typed` and press Enter.
    fn bar_type(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2, typed: &str) {
        let key = |key, modifiers| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
        bar_click(app, ctx, at);
        bar_frame(app, ctx, key(egui::Key::A, egui::Modifiers::COMMAND));
        bar_frame(app, ctx, vec![egui::Event::Text(typed.into())]);
        bar_frame(app, ctx, key(egui::Key::Enter, egui::Modifiers::NONE));
        bar_frame(app, ctx, vec![]);
    }

    /// The Control bar's clickable widgets of the last frame that lie between x `from` and `to`,
    /// left to right.
    fn bar_buttons(ctx: &egui::Context, from: f32, to: f32) -> Vec<egui::Rect> {
        let mut all: Vec<egui::Rect> = ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.rect.left() >= from && w.rect.right() <= to)
                .map(|w| w.rect)
                .collect()
        });
        all.sort_by(|a, b| a.left().total_cmp(&b.left()));
        all
    }

    /// The artboards' rectangles, the undo steps and the artboards' names.
    fn boards(app: &VectorcraftApp) -> (Vec<vectorcraft_geom::Rect>, usize, Vec<String>) {
        let st = app.session.active().unwrap();
        (st.doc.artboards.iter().map(|a| a.rect).collect(), st.history.undo.len(), st.doc.artboards.iter().map(|a| a.name.clone()).collect())
    }

    /// With the Artboard tool in use, the Control bar shows the active artboard's controls in place
    /// of the object controls: its preset, name, the Transform link, Move and Scale Artwork with
    /// Artboard, X, Y, W and H, left to right, and in a narrow window the link without the fields.
    /// Another tool shows the object controls.
    #[test]
    fn the_control_bar_shows_the_artboard_tools_controls() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "units": "Points"})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let ctx = bar_context();
        let has = |texts: &[(String, egui::Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
        let texts = bar_texts(&mut app, &ctx);
        assert!(has(&texts, "Opacity:") && !has(&texts, "Name:") && !has(&texts, "Preset:"), "{texts:?}");
        app.select_tool("artboard");
        let texts = bar_texts(&mut app, &ctx);
        let order = [
            "Artboard",
            "Preset:",
            "Name:",
            "Artboard 1",
            "Transform",
            "Move Artwork with Artboard",
            "Scale Artwork with Artboard",
            "X:",
            "Y:",
            "W:",
            "200 pt",
            "H:",
        ];
        let lefts: Vec<f32> = order.iter().map(|s| text_rect(&texts, s).left()).collect();
        assert!(lefts.windows(2).all(|w| w[0] < w[1]), "{order:?} at {lefts:?}");
        for s in ["Opacity:", "Stroke:"] {
            assert!(!has(&texts, s), "{s}: {texts:?}");
        }
        // In a window that leaves 300 points right of Scale Artwork with Artboard, the inline fields
        // go (X: would start inside the window).
        let width = text_rect(&texts, "Scale Artwork with Artboard").right() + 300.0;
        bar_frame_in(&mut app, &ctx, vec![], width);
        let texts = bar_frame_in(&mut app, &ctx, vec![], width);
        assert!(has(&texts, "Scale Artwork with Artboard") && !has(&texts, "X:") && !has(&texts, "W:"), "{texts:?}");
        // In the app's narrowest window, 800 points wide, the name and the Transform link still fit,
        // and a click on the link shows the artboard's X, Y, W and H in the Transform panel's popover.
        let narrow = |app: &mut VectorcraftApp, events| bar_frame_in(app, &ctx, events, 800.0);
        narrow(&mut app, vec![]);
        let texts = narrow(&mut app, vec![]);
        let link = text_rect(&texts, "Transform");
        assert!(link.right() < 790.0 && text_rect(&texts, "Artboard 1").right() < link.left(), "{texts:?}");
        // Move and Scale Artwork with Artboard show as their boxes alone, inside the window, and the
        // second one turns Scale Artwork with Artboard on.
        assert!(!has(&texts, "Move Artwork with Artboard"), "{texts:?}");
        let boxes = bar_buttons(&ctx, link.right(), 790.0);
        assert_eq!(boxes.len(), 2, "{boxes:?}");
        let scale = |app: &VectorcraftApp| app.session.tool_options()["scaleArt"].as_bool();
        assert_eq!(scale(&app), Some(false));
        let press = |pressed| egui::Event::PointerButton {
            pos: boxes[1].center(),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        narrow(&mut app, vec![egui::Event::PointerMoved(boxes[1].center()), press(true)]);
        narrow(&mut app, vec![press(false)]);
        assert_eq!(scale(&app), Some(true));
        narrow(&mut app, vec![]);
        let button =
            |pressed| egui::Event::PointerButton { pos: link.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        narrow(&mut app, vec![egui::Event::PointerMoved(link.center()), button(true)]);
        narrow(&mut app, vec![button(false)]);
        let texts = narrow(&mut app, vec![]);
        assert!(has(&texts, "W:") && has(&texts, "200 pt"), "the popover: {texts:?}");
    }

    /// The Control bar's W resizes the active artboard about the reference point in one undo step:
    /// about its center at first, and from its top-left corner once the locator's top-left point is
    /// chosen.
    #[test]
    fn the_control_bar_resizes_the_active_artboard_about_the_reference_point() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "units": "Points", "artboards": 2})).unwrap();
        app.select_tool("artboard");
        select(&mut app, 1);
        let ctx = bar_context();
        let (before, undo, _) = boards(&app);
        let w = text_rect(&bar_texts(&mut app, &ctx), "W:");
        bar_type(&mut app, &ctx, egui::pos2(w.right() + 40.0, w.center().y), "300");
        let b = before[1];
        let mut want = before.clone();
        want[1] = vectorcraft_geom::Rect::new(b.x0 - 50.0, b.y0, b.x1 + 50.0, b.y1);
        assert_eq!(boards(&app).0, want, "Artboard 2 widens about its center");
        assert_eq!(boards(&app).1, undo + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app).0, before);
        // The locator's points are the 9-point-square buttons; the top-left one is drawn first.
        let texts = bar_texts(&mut app, &ctx);
        let points: Vec<egui::Rect> = bar_buttons(&ctx, 0.0, text_rect(&texts, "X:").left())
            .into_iter()
            .filter(|r| (r.width() - 9.0).abs() < 0.5 && (r.height() - 9.0).abs() < 0.5)
            .collect();
        let top_left = points.iter().min_by(|a, b| (a.top(), a.left()).partial_cmp(&(b.top(), b.left())).unwrap()).unwrap();
        bar_click(&mut app, &ctx, top_left.center());
        assert_eq!(ctx.data(|d| d.get_temp::<usize>(egui::Id::new("refpt"))), Some(0), "the shared reference point is the top-left one");
        let w = text_rect(&bar_texts(&mut app, &ctx), "W:");
        bar_type(&mut app, &ctx, egui::pos2(w.right() + 40.0, w.center().y), "300");
        want[1] = vectorcraft_geom::Rect::new(b.x0, b.y0, b.x0 + 300.0, b.y1);
        assert_eq!(boards(&app).0, want, "Artboard 2 widens from its left edge");
    }

    /// The Control bar's Portrait and Landscape buttons swap the active artboard's width and height,
    /// its top-left corner staying put, in one undo step each.
    #[test]
    fn the_control_bar_orientation_swaps_width_and_height() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "units": "Points"})).unwrap();
        app.select_tool("artboard");
        let ctx = bar_context();
        let (_, undo, _) = boards(&app);
        // Between "Preset:" and "Name:", the buttons that take a click: the preset menu, Portrait,
        // Landscape and New Artboard (Delete Artboard is disabled with one artboard).
        let buttons = |app: &mut VectorcraftApp| {
            let texts = bar_texts(app, &ctx);
            bar_buttons(&ctx, text_rect(&texts, "Preset:").right(), text_rect(&texts, "Name:").left())
        };
        let b = buttons(&mut app);
        bar_click(&mut app, &ctx, b[b.len() - 3].center());
        assert_eq!((boards(&app).0, boards(&app).1), (vec![vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 200.0)], undo + 1), "Portrait");
        let b = buttons(&mut app);
        bar_click(&mut app, &ctx, b[b.len() - 2].center());
        assert_eq!((boards(&app).0, boards(&app).1), (vec![vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 100.0)], undo + 2), "Landscape");
    }

    /// Typing a name into the Control bar's Name field and pressing Enter renames the active
    /// artboard alone, in one undo step.
    #[test]
    fn the_control_bar_name_field_renames_the_active_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
        app.select_tool("artboard");
        select(&mut app, 1);
        let ctx = bar_context();
        let (_, undo, _) = boards(&app);
        let name = text_rect(&bar_texts(&mut app, &ctx), "Artboard 2");
        bar_type(&mut app, &ctx, name.center(), "Cover");
        assert_eq!((boards(&app).2, boards(&app).1), (vec!["Artboard 1".to_string(), "Cover".to_string()], undo + 1));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app).2, ["Artboard 1", "Artboard 2"]);
    }

    /// The Control bar's New Artboard adds an artboard and makes it the active one, Artboard Options
    /// opens the active artboard's options, Rearrange All Artboards opens its dialog, and Delete
    /// Artboard deletes the selected artboard.
    #[test]
    fn the_control_bar_adds_deletes_and_opens_artboard_dialogs() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.select_tool("artboard");
        let ctx = bar_context();
        let texts = bar_texts(&mut app, &ctx);
        // With one artboard Delete Artboard is disabled, so New Artboard is the last button before
        // "Name:"; with two, Delete Artboard is.
        let (preset, name) = (text_rect(&texts, "Preset:").right(), text_rect(&texts, "Name:").left());
        let b = bar_buttons(&ctx, preset, name);
        bar_click(&mut app, &ctx, b[b.len() - 1].center());
        assert_eq!((boards(&app).2.len(), selected(&app, 2)), (2, 1), "New Artboard adds Artboard 2 and makes it active");
        // Artboard Options and Rearrange All Artboards: the two buttons between the name and the
        // Transform link.
        let texts = bar_texts(&mut app, &ctx);
        let between =
            |texts: &[(String, egui::Rect)]| bar_buttons(&ctx, text_rect(texts, "Artboard 2").right(), text_rect(texts, "Transform").left());
        let options = between(&texts)[0];
        bar_click(&mut app, &ctx, options.center());
        let d = app.ui.dialog.take().unwrap();
        assert_eq!((d.kind.as_str(), d.f64("index", 9.0)), ("artboardOptions", 1.0));
        let texts = bar_texts(&mut app, &ctx);
        let rearrange = between(&texts)[1];
        bar_click(&mut app, &ctx, rearrange.center());
        assert_eq!(app.ui.dialog.take().map(|d| d.kind), Some(crate::dialogs::rearrange_artboards::KIND.to_string()));
        bar_texts(&mut app, &ctx);
        let b = bar_buttons(&ctx, preset, name);
        bar_click(&mut app, &ctx, b[b.len() - 1].center());
        assert_eq!(boards(&app).2, ["Artboard 1"], "Delete Artboard deletes Artboard 2");
    }

    /// Choosing the Artboard tool starts it on the active artboard, the one the panel highlights:
    /// Artboard Options shows that artboard, the pointer moving over another artboard leaves it
    /// active, and Delete removes it. Shift+O while the tool is in use keeps the artboards it has
    /// selected.
    #[test]
    fn choosing_the_artboard_tool_starts_it_on_the_active_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        for x in [400, 800] {
            app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
        }
        select(&mut app, 1);
        app.run("tool.select", json!({"tool": "artboard"})).unwrap();
        assert_eq!(app.session.tool_options()["active"], json!(1));
        app.run("tool.options", json!({"tool": "artboard"})).unwrap();
        assert_eq!(app.ui.dialog.take().unwrap().str("name"), "Artboard 2");
        let view = app.view_info();
        crate::canvas::dispatch(&mut app, &vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Move, 100.0, 75.0), view);
        assert_eq!(selected(&app, 3), 1, "the pointer over Artboard 1 leaves Artboard 2 active");
        let r = app.session.tool_key(vectorcraft_tools::ToolKey::Delete, Default::default(), view);
        crate::canvas::apply_requests(&mut app, r);
        let names: Vec<String> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
        assert_eq!(names, ["Artboard 1", "Artboard 3"], "Delete removes Artboard 2");
        // Shift+O while the tool is in use keeps the artboards it has selected.
        app.session.set_tool_option("selected", &json!([0, 1]));
        let ctx = egui::Context::default();
        let shift_o = vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT), egui::Event::Text("O".into())];
        let mut out = ctx.run_ui(egui::RawInput { events: shift_o, ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert_eq!((app.session.tool_id(), picked(&app, 2)), ("artboard", vec![0, 1]));
    }

    /// A row dragged onto New Artboard duplicates its artboard with its art (#446), outlining the
    /// button while it is held over it, and the copy becomes the active artboard.
    #[test]
    fn dragging_a_row_onto_new_artboard_duplicates_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        let shapes = frame(&mut app, vec![]);
        let t = Tokens::get(&ctx);
        let rect_of = |fill| {
            shapes.iter().find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == fill => Some(r.rect),
                _ => None,
            })
        };
        let row = rect_of(t.row_selected).unwrap().center();
        // The bottom bar's buttons sit under its divider, New Artboard second from the right.
        let bar = rect_of(t.divider).unwrap();
        let new = pos2(bar.right() - 24.0 - 4.0 - 12.0, bar.bottom() + 2.0 + 12.0);
        let button =
            |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(row), button(row, true)]);
        frame(&mut app, vec![egui::Event::PointerMoved(row + vec2(0.0, 12.0))]);
        let held = frame(&mut app, vec![egui::Event::PointerMoved(new)]);
        assert!(held.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.stroke.color == t.accent)), "New Artboard is outlined");
        frame(&mut app, vec![button(new, false)]);
        let d = &app.session.active().unwrap().doc;
        assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (2, 2), "the artboard and its art");
        assert_eq!(app.view().unwrap().artboard, 1);
    }

    /// A click in a row's lock column locks its artboard and shows the lock next to the name, a
    /// second click unlocks it, and each click is one undo step. Delete Artboard leaves a locked
    /// artboard.
    #[test]
    fn the_lock_column_locks_and_unlocks_an_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 400, "y": 0, "width": 200, "height": 150})).unwrap();
        select(&mut app, 1);
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        // The highlighted row (Artboard 2's): its lock column is 36 points in from its right end.
        let shapes = frame(&mut app, 0.0, vec![]);
        let row = shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).row_selected => Some(r.rect),
                _ => None,
            })
            .unwrap();
        let lock = egui::pos2(row.right() - 36.0, row.center().y);
        let click = |at: egui::Pos2| {
            let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            vec![egui::Event::PointerMoved(at), b(true), b(false)]
        };
        let away = vec![egui::Event::PointerMoved(egui::pos2(-50.0, -50.0))];
        let locked = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.locked).collect::<Vec<_>>();
        // Whether something is drawn in the lock column of Artboard 2's row.
        let cell = egui::Rect::from_center_size(lock, vec2(14.0, 14.0));
        let lock_drawn = |shapes: Vec<egui::epaint::ClippedShape>| {
            shapes.iter().any(|c| {
                let b = c.shape.visual_bounding_rect();
                b.is_positive() && cell.contains_rect(b)
            })
        };
        assert!(!lock_drawn(frame(&mut app, 1.0, away.clone())));
        frame(&mut app, 2.0, click(lock));
        assert_eq!(locked(&app), [false, true]);
        assert!(lock_drawn(frame(&mut app, 3.0, away.clone())), "the lock shows");
        assert!(deletable(&app, 2).is_empty(), "Delete Artboard leaves it");
        delete_selected(&mut app);
        assert_eq!(app.session.active().unwrap().doc.artboards.len(), 2);
        frame(&mut app, 5.0, click(lock));
        assert_eq!(locked(&app), [false, false]);
        assert!(!lock_drawn(frame(&mut app, 6.0, away)));
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(locked(&app), [false, true], "one undo step");
    }

    /// Alt-click on New Artboard opens Artboard Options for the new artboard and adds nothing yet:
    /// the fields are those a plain click's artboard gets. Cancel adds nothing; OK adds the artboard
    /// with the fields in one undo step and makes it the active one.
    #[test]
    fn alt_clicking_new_artboard_opens_artboard_options_for_the_new_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 100, "height": 50})).unwrap();
        select(&mut app, 0);
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        // The bottom bar's buttons sit under its divider, New Artboard second from the right.
        let bar = frame(&mut app, 0.0, vec![])
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).divider => Some(r.rect),
                _ => None,
            })
            .unwrap();
        let new = pos2(bar.right() - 24.0 - 4.0 - 12.0, bar.bottom() + 2.0 + 12.0);
        let click = |modifiers: egui::Modifiers| {
            let b = |pressed| egui::Event::PointerButton { pos: new, button: egui::PointerButton::Primary, pressed, modifiers };
            vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(new), b(true), b(false)]
        };
        let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.len();
        let fields = |app: &VectorcraftApp| {
            let d = app.ui.dialog.as_ref().unwrap();
            assert_eq!(d.kind, crate::dialogs::artboard_options::KIND);
            (d.str("name"), ["x", "y", "width", "height"].map(|k| d.f64(k, -1.0)))
        };
        // Alt-click: the dialog for Artboard 3, 20 pt right of Artboard 2 and as large; no artboard yet.
        frame(&mut app, 1.0, click(egui::Modifiers::ALT));
        assert_eq!(fields(&app), ("Artboard 3".to_string(), [520.0, 300.0, 100.0, 50.0]));
        assert_eq!(count(&app), 2, "no artboard is added before OK");
        // A plain click adds the artboard the dialog showed, with no dialog.
        crate::dialogs::cancel(&mut app);
        frame(&mut app, 2.0, click(egui::Modifiers::NONE));
        let ab = app.session.active().unwrap().doc.artboards[2].clone();
        assert_eq!((ab.name.as_str(), ab.rect), ("Artboard 3", vectorcraft_geom::Rect::new(520.0, 300.0, 620.0, 350.0)));
        assert!(app.ui.dialog.is_none());
        app.run("edit.undo", json!({})).unwrap();
        select(&mut app, 0);
        // Cancel adds nothing.
        frame(&mut app, 3.0, click(egui::Modifiers::ALT));
        crate::dialogs::cancel(&mut app);
        assert_eq!((count(&app), app.view().unwrap().artboard), (2, 0));
        // OK adds it with the fields, in one undo step, and makes it the active artboard.
        frame(&mut app, 4.0, click(egui::Modifiers::ALT));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("name".into(), json!("Cover"));
        d.fields.insert("width".into(), json!(300));
        let undo = app.session.doc().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        let ab = app.session.active().unwrap().doc.artboards[2].clone();
        assert_eq!((ab.name.as_str(), ab.rect), ("Cover", vectorcraft_geom::Rect::new(520.0, 300.0, 820.0, 350.0)));
        assert_eq!((app.view().unwrap().artboard, app.session.doc().unwrap().history.undo.len()), (2, undo + 1));
        assert!(app.ui.dialog.is_none());
    }

    /// `ui.newArtboard` answers with the dialog's kind. A name left empty gives the new artboard
    /// its default name, and with the Artboard tool in use the tool moves to the new artboard.
    #[test]
    fn new_artboard_options_with_an_empty_name_add_the_default_name() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(app.run("ui.newArtboard", json!({})).is_err(), "no document");
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        app.select_tool("artboard");
        assert_eq!(app.run("ui.newArtboard", json!({})).unwrap(), json!({"dialog": "artboardOptions"}));
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!(" "));
        crate::dialogs::confirm(&mut app).unwrap();
        let names: Vec<String> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
        assert_eq!(names, ["Artboard 1", "Artboard 2"]);
        assert_eq!((selected(&app, 2), app.session.tool_options()["active"].clone()), (1, json!(1)));
    }
}
