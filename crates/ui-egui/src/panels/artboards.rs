//! Artboards panel: numbered list with inline rename (double-click the name), move up / down, new,
//! delete; a row dragged onto New Artboard duplicates its artboard (as Duplicate Artboards does, with
//! its art when the Artboard tool's Move/Copy Artwork with Artboard is on). The highlighted row is the active artboard, the one the status bar's navigator shows and
//! Fit Artboard in Window fits: clicking a row makes it active, double-clicking its number also
//! fits it in the window. The options icon at a row's right end opens Artboard Options: a click on
//! the active artboard's icon, or a double-click on another row's icon.

use egui::{Sense, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_geom::Rect;

use super::{pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, icons};

/// The Artboard tool's Move Artwork with Artboard and Scale Artwork with Artboard options (#602) as
/// check boxes: the Control bar and Properties show them while the tool is in use.
pub(crate) fn art_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    let opts = app.session.tool_options();
    for (key, label, default) in [("moveArt", tl!("Move Artwork with Artboard"), true), ("scaleArt", tl!("Scale Artwork with Artboard"), false)] {
        let on = opts[key].as_bool().unwrap_or(default);
        if widgets::check(ui, label, on, true) {
            app.run("tool.setOption", json!({ "key": key, "value": !on })).ok();
        }
    }
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

/// The artboards a new size for artboard `i` goes to: the selected artboards ([`picked`]) when
/// `i` is one of them, else `i` alone.
pub(crate) fn resized_with(app: &VectorcraftApp, i: usize) -> Vec<usize> {
    let all = picked(app, app.session.active().map_or(0, |d| d.doc.artboards.len()));
    if all.contains(&i) { all } else { vec![i] }
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

/// Delete the picked artboards and select a remaining one.
pub(crate) fn delete_selected(app: &mut VectorcraftApp) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let mut all = picked(app, n);
    if n > 1 && all.len() == n {
        // Deleting all artboards keeps artboard 1 (the public Delete artboards guide).
        all.retain(|i| *i != 0);
    }
    if n > all.len() && app.run("artboard.delete", json!({ "indices": all })).is_ok() {
        select(app, all.first().copied().unwrap_or(0).saturating_sub(1));
    }
}

/// Shift- or Ctrl/Cmd-click on artboard `i`'s row while the Artboard tool is in use: into its
/// selection or out of it (the active one stays the navigator's).
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

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let abs: Vec<String> = app.session.active().map(|d| d.doc.artboards.iter().map(|a| a.name.clone()).collect()).unwrap_or_default();
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
                let name_rect = egui::Rect::from_min_max(pos2(r.left() + 32.0, r.top() + 2.0), pos2(r.right() - 28.0, r.bottom() - 2.0));
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
                let several = ui.input(|inp| inp.modifiers.shift || inp.modifiers.command);
                if resp.clicked() && several && app.session.tool_id() == "artboard" {
                    toggle(app, i, abs.len());
                } else if resp.clicked() {
                    select(app, i);
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
        } else if new.clicked() && app.run("artboard.new", json!({})).is_ok() {
            select(app, n);
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Artboard"), false, n > 1, 24.0).clicked() {
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
        duplicate(app, sel);
    }
    if menu_item(ui, tl!("Delete Artboards"), n > 1, false) {
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

    /// With the Artboard tool, Shift-click on a row adds its artboard to the tool's selection or
    /// takes it out (#1139), and every selected row is highlighted.
    #[test]
    fn shift_clicking_rows_selects_several_artboards() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        for x in [400, 800] {
            app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
        }
        app.select_tool("artboard");
        select(&mut app, 0);
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let raw = egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        let shapes = frame(&mut app, 0.0, vec![]);
        let third = shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == "Artboard 3" => Some(t.pos + vec2(4.0, 4.0)),
                _ => None,
            })
            .unwrap();
        let click = |at: egui::Pos2| {
            let b =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::SHIFT };
            vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT), egui::Event::PointerMoved(at), b(true), b(false)]
        };
        frame(&mut app, 1.0, click(third));
        assert_eq!(picked(&app, 3), [0, 2]);
        let rows = frame(&mut app, 2.0, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)])
            .iter()
            .filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).row_selected))
            .count();
        assert_eq!(rows, 2, "both rows highlighted");
        frame(&mut app, 3.0, click(third));
        assert_eq!(picked(&app, 3), [0]);
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
}
