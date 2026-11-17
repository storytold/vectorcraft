//! Variables panel: the document's variables (what they drive, which objects) and its
//! datasets. Clicking a dataset applies it; clicking a variable selects what it binds; the
//! arrows step to the next or previous one. Everything the reference panel does here runs a
//! `variable.*` / `dataset.*` command, so the panel and an agent drive the document the same
//! way — the commands are also in Window › Variables, the palette and over MCP.
//!
//! A row's name goes in italics when an edit has moved the art away from what the row says:
//! the document then no longer matches the active dataset, and "Update Data Set" writes the
//! art back into it.

use egui::{Rect, Response, Sense, Ui, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind, VariableKind};
use vectorcraft_engine::DocState;
use vectorcraft_engine::cmd::{VariableSort, VariableSortKey, variable_order};

use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

/// The panel id, as the dock, the Window menu and `window.panel` name it.
pub const ID: &str = "variables";

/// The width of the variable list's type column, which holds each row's kind icon.
const TYPE_COLUMN: f32 = 22.0;

/// The width of the Variables column in a list `width` wide; the Objects column takes the rest.
fn name_column(width: f32) -> f32 {
    ((width - TYPE_COLUMN) / 2.0).max(0.0)
}

/// The sort a click on `key`'s header asks for: that column in ascending order, or the reverse
/// of the order the list has when it is already sorted by that column.
fn clicked_sort(sort: VariableSort, key: VariableSortKey) -> VariableSort {
    VariableSort { by: key, descending: sort.by == key && !sort.descending }
}

/// The arrow a header shows while the list is sorted by its column.
fn sort_arrow(sort: VariableSort, key: VariableSortKey) -> Option<&'static str> {
    (sort.by == key).then_some(if sort.descending { "▼" } else { "▲" })
}

/// Draw the list's three column headers (type, Variables, Objects); a click on one returns the
/// sort it asks for.
fn headers(ui: &mut Ui, sort: VariableSort) -> Option<VariableSort> {
    let t = Tokens::get(ui.ctx());
    let h = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let name_w = name_column(rect.width());
    let name_left = rect.left() + TYPE_COLUMN;
    let cells = [
        (VariableSortKey::Type, tl!("Type"), Rect::from_min_max(rect.min, pos2(name_left, rect.bottom()))),
        (VariableSortKey::Name, tl!("Variables"), Rect::from_min_max(pos2(name_left, rect.top()), pos2(name_left + name_w, rect.bottom()))),
        (VariableSortKey::Object, tl!("Objects"), Rect::from_min_max(pos2(name_left + name_w, rect.top()), rect.max)),
    ];
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut clicked = None;
    for (key, label, cell) in cells {
        let r = ui.interact(cell, ui.scope_id().with(("variables-sort", key.id())), Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(cell, 0.0, t.hover);
        }
        let arrow = sort_arrow(sort, key);
        let p = ui.painter().with_clip_rect(cell);
        if key == VariableSortKey::Type {
            // The type column is too narrow for a word: its icon, or the arrow once it sorts.
            match arrow {
                Some(a) => {
                    p.text(cell.center(), egui::Align2::CENTER_CENTER, a, font.clone(), t.text_dim);
                }
                None => crate::icons::paint(ui, "arrow-up-down", Rect::from_center_size(cell.center(), vec2(14.0, 14.0)), t.icon),
            }
        } else {
            p.text(cell.left_center() + vec2(4.0, 0.0), egui::Align2::LEFT_CENTER, label, font.clone(), t.text_strong);
            if let Some(a) = arrow {
                p.text(cell.right_center() - vec2(4.0, 0.0), egui::Align2::RIGHT_CENTER, a, font.clone(), t.text_dim);
            }
        }
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, arrow.is_some(), label));
        if r.on_hover_text(label).clicked() {
            clicked = Some(clicked_sort(sort, key));
        }
    }
    clicked
}

/// Draw one variable row across the three columns: its kind's icon, its name and the object it
/// drives. Each column clips its text to its own width.
fn variable_row(ui: &mut Ui, row: &VarRow, selected: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let h = ui.spacing().interact_size.y;
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, t.row_selected);
    } else if r.hovered() {
        ui.painter().rect_filled(rect, 0.0, t.hover);
    }
    // The kind's own icon, so a text variable and a visibility one read apart.
    let icon = match row.kind {
        VariableKind::Text => "type",
        VariableKind::Visibility => "eye",
        VariableKind::Image => "image",
        VariableKind::Graph => "chart-column",
        VariableKind::Unknown => "square-dashed",
    };
    let icon_rect = Rect::from_center_size(pos2(rect.left() + TYPE_COLUMN / 2.0, rect.center().y), vec2(14.0, 14.0));
    crate::icons::paint(ui, icon, icon_rect, t.icon);
    let name_left = rect.left() + TYPE_COLUMN;
    let name_right = name_left + name_column(rect.width());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let (pad, left) = (vec2(4.0, 0.0), egui::Align2::LEFT_CENTER);
    let name_cell = Rect::from_min_max(pos2(name_left, rect.top()), pos2(name_right - 4.0, rect.bottom()));
    ui.painter().with_clip_rect(name_cell).text(name_cell.left_center() + pad, left, &row.name, font.clone(), t.text);
    let object_cell = Rect::from_min_max(pos2(name_right, rect.top()), rect.max);
    ui.painter().with_clip_rect(object_cell).text(object_cell.left_center() + pad, left, &row.object, font, t.text_dim);
    let full = if row.object.is_empty() { row.name.clone() } else { format!("{} · {}", row.name, row.object) };
    widgets::list_item_info(&r, selected, || full.clone());
    // A column clips a long name, so the whole of it shows on hover.
    r.on_hover_text(full)
}

/// One variable row: its kind's icon, its name, and the object it drives.
struct VarRow {
    name: String,
    kind: VariableKind,
    bound: Vec<u64>,
    /// The name of the object it drives, or "Multiple objects" when it drives more than one.
    object: String,
}

/// One dataset row: its name and how many values it carries.
struct SetRow {
    name: String,
    values: usize,
    active: bool,
    /// False once an edit has moved the art away from what this row says.
    matches: bool,
}

/// The row the panel acts on: Delete, Options… and Select Bound Object all mean it. A name
/// the document no longer defines highlights nothing (deleted from the palette, undone).
fn highlighted_row(st: &DocState) -> Option<String> {
    st.variables_highlight.clone().filter(|h| st.doc.variables.variable(h).is_some())
}

/// What a variable drives, as object ids: what Select Bound Object selects.
fn bound_objects(st: &DocState, name: &str) -> Vec<u64> {
    st.doc.variables.objects_of(name).iter().map(|i| i.0).collect()
}

/// A row click: highlight the row, and select what it drives (its bindings are left alone).
pub(crate) fn click_row(app: &mut VectorcraftApp, name: &str, bound: &[u64]) {
    app.run("variable.highlight", json!({"variable": name})).ok();
    if !bound.is_empty() {
        app.run("select.set", json!({"ids": bound})).ok();
    }
}

/// Select Bound Object: select what the highlighted variable drives; its bindings stay.
pub(crate) fn select_bound_object(app: &mut VectorcraftApp) {
    let ids = app.session.active().map(|st| highlighted_row(st).map(|h| bound_objects(st, &h))).unwrap_or_default().unwrap_or_default();
    if !ids.is_empty() {
        app.run("select.set", json!({"ids": ids})).ok();
    }
}

/// What the selection allows the bottom bar's and the panel menu's commands to do.
struct Picked {
    selected: Vec<u64>,
    has_selection: bool,
    /// Unbind needs a bound object under the selection, not just any selection.
    selection_bound: bool,
    /// Make Text Dynamic needs exactly one type object: a text variable only drives type.
    single_type: bool,
    /// Make Image Dynamic needs exactly one image object.
    single_image: bool,
    /// Make Graph Dynamic needs a selection of graph roots only.
    selection_graph: bool,
}

fn picked(st: &vectorcraft_engine::DocState) -> Picked {
    let selected: Vec<u64> = st.selection.objects.iter().map(|i| i.0).collect();
    let selection_bound = selected.iter().any(|id| st.doc.variables.bindings.contains_key(&NodeId(*id)));
    let single_type = matches!(selected.as_slice(), [one] if st.doc.node(NodeId(*one)).is_some_and(|n| matches!(n.kind, NodeKind::Text(_))));
    let single_image = matches!(selected.as_slice(), [one] if st.doc.node(NodeId(*one)).is_some_and(|n| matches!(n.kind, NodeKind::Image(_))));
    let selection_graph = !selected.is_empty() && selected.iter().all(|id| st.doc.node(NodeId(*id)).is_some_and(|n| n.extras().graph.is_some()));
    Picked { has_selection: !selected.is_empty(), selected, selection_bound, single_type, single_image, selection_graph }
}

/// Delete Variable: the highlighted variable only (the command's default); the rest stay.
pub(crate) fn delete_highlighted(app: &mut VectorcraftApp) {
    app.run("variable.delete", json!({})).ok();
}

/// Delete Data Set: the current one only (the command's default); the rest of the rows stay.
pub(crate) fn delete_active_dataset(app: &mut VectorcraftApp) {
    app.run("dataset.delete", json!({})).ok();
}

/// The Data Set box: a new name typed over the current data set's renames it with
/// `dataset.rename`, in one undo step. The same name runs nothing, so no undo step is added; an
/// empty or taken name is refused, and the status bar says why.
pub(crate) fn rename_active_dataset(app: &mut VectorcraftApp, from: &str, to: &str) {
    if from.is_empty() || to.trim() == from {
        return;
    }
    if let Err(e) = app.run("dataset.rename", json!({"name": from, "newName": to})) {
        app.status(e);
    }
}

/// Draw document variables, data sets and their two-row footer of capture and binding controls.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        widgets::dim_label(ui, tl!("No document"));
        return;
    };
    // Owned snapshots: the rows below run commands, which borrow the app mutably.
    let xml_id = app.session.prefs.identify_objects_by == "xmlId";
    // In the order `variable.sort` set, which `variable.list` reports too.
    let all = &st.doc.variables.variables;
    let vars: Vec<VarRow> = variable_order(st, xml_id)
        .into_iter()
        .filter_map(|i| all.get(i))
        .map(|v| {
            let bound: Vec<u64> = st.doc.variables.objects_of(&v.name).iter().map(|i| i.0).collect();
            let object = match bound.as_slice() {
                [] => String::new(),
                [one] => st.doc.node(NodeId(*one)).map(|n| n.identify_label(xml_id)).unwrap_or_default(),
                _ => tl!("Multiple objects").to_string(),
            };
            VarRow { name: v.name.clone(), kind: v.kind, bound, object }
        })
        .collect();
    // What the art holds now, read once for every row's drift check.
    let current = st.doc.captured_values();
    let sets: Vec<SetRow> = st
        .doc
        .variables
        .datasets
        .iter()
        .map(|d| SetRow {
            name: d.name.clone(),
            values: d.values.len(),
            active: st.doc.variables.active_dataset.as_deref() == Some(d.name.as_str()),
            matches: d.matches(&current),
        })
        .collect();
    let active = sets.iter().find(|s| s.active);
    let Picked { selected, has_selection, selection_bound, single_type, single_image, selection_graph } = picked(st);
    let highlighted = highlighted_row(st);
    // Lock Variables: no variable is defined, deleted or renamed until they are unlocked.
    let locked = st.doc.variables.locked;
    let sort = st.variables_sort;

    // The current data set across the top: its name in a box that renames it, the list that
    // chooses another, and the arrows that step through them.
    ui.horizontal(|ui| {
        widgets::field_label(ui, tl!("Data Set"));
        let names: Vec<&str> = sets.iter().map(|s| s.name.as_str()).collect();
        let current = active.map(|s| s.name.as_str()).unwrap_or("");
        let typed = ui.add_enabled_ui(active.is_some(), |ui| widgets::text_field(ui, "variables-active-name", Some(current), 134.0, 1)).inner;
        if let Some(to) = typed {
            rename_active_dataset(app, current, &to);
        }
        // The closed list button is too narrow for a name and shows none; the open list marks the
        // current data set.
        let picked = widgets::combo_named(ui, "variables-active", Some(tl!("Data Sets")), "", 26.0, |ui| {
            let mut chosen = None;
            for (i, name) in names.iter().enumerate() {
                if ui.add(egui::Button::selectable(*name == current, *name)).clicked() {
                    chosen = Some(i);
                }
            }
            chosen
        });
        if let Some(name) = picked.and_then(|i| names.get(i))
            && *name != current
        {
            app.run("dataset.select", json!({"name": name})).ok();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let has = !sets.is_empty();
            if widgets::icon_button_enabled(ui, "chevron-left", tl!("Previous Data Set"), false, has, 24.0).clicked() {
                app.run("dataset.prev", json!({})).ok();
            }
            if widgets::icon_button_enabled(ui, "chevron-right", tl!("Next Data Set"), false, has, 24.0).clicked() {
                app.run("dataset.next", json!({})).ok();
            }
        });
    });

    // The list's column headers: a click sorts the list by that column, and a second click
    // reverses the order.
    if let Some(next) = headers(ui, sort) {
        app.run("variable.sort", json!({"by": next.by.id(), "descending": next.descending})).ok();
    }
    if vars.is_empty() {
        widgets::dim_label(ui, tl!("None"));
    }
    for row in &vars {
        let r = variable_row(ui, row, highlighted.as_deref() == Some(row.name.as_str()));
        // A click highlights the row and selects what it drives; a double click opens
        // Variable Options for it.
        if r.clicked() {
            click_row(app, &row.name, &row.bound);
        }
        if r.double_clicked() && !locked {
            crate::dialogs::variables::open(app, "variable.rename", Some(&row.name));
        }
    }

    widgets::subheader(ui, tl!("Data Sets"));
    if sets.is_empty() {
        widgets::dim_label(ui, tl!("None"));
    }
    for row in &sets {
        // The active row is ticked and selected; a row the art has drifted from is in
        // italics, so an edit that left the document not matching it is visible.
        let label = format!("{}{} · {}", if row.active { "● " } else { "" }, row.name, row.values);
        let mut text = egui::RichText::new(label);
        if !row.matches {
            text = text.italics();
        }
        if ui.selectable_label(row.active, text).clicked() && !row.active {
            app.run("dataset.select", json!({"name": row.name})).ok();
        }
    }

    // The bottom bar: make the selection dynamic, define and delete what the lists show, and
    // lock the variables.
    let footer_height = 48.0 + 2.0 * ui.spacing().item_spacing.y + 1.0;
    widgets::panel_footer(ui, footer_height, |ui| {
        ui.add(egui::Separator::default().spacing(1.0));
        ui.horizontal(|ui| {
            if widgets::icon_button_enabled(ui, "type", tl!("Make Text Dynamic"), false, single_type && !locked, 24.0).clicked() {
                app.run("variable.makeTextDynamic", json!({})).ok();
            }
            if widgets::icon_button_enabled(ui, "eye", tl!("Make Visibility Dynamic"), false, has_selection && !locked, 24.0).clicked() {
                app.run("variable.makeVisibilityDynamic", json!({})).ok();
            }
            if widgets::icon_button_enabled(ui, "image", tl!("Make Image Dynamic"), false, single_image && !locked, 24.0).clicked() {
                app.run("variable.makeImageDynamic", json!({})).ok();
            }
            if widgets::icon_button_enabled(ui, "chart-column", tl!("Make Graph Dynamic"), false, selection_graph && !locked, 24.0).clicked() {
                app.run("variable.makeGraphDynamic", json!({})).ok();
            }
            if widgets::icon_button_enabled(ui, "plus", tl!("New Variable…"), false, !locked, 24.0).clicked() {
                crate::menus::invoke(app, "variable.define", json!({}));
            }
            if widgets::icon_button_enabled(ui, "link-2-off", tl!("Unbind Variable"), false, selection_bound, 24.0).clicked() {
                app.run("variable.unbind", json!({"ids": selected})).ok();
            }
            // The bin deletes the highlighted variable only, as the reference panel's does.
            if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Variable"), false, highlighted.is_some() && !locked, 24.0).clicked() {
                delete_highlighted(app);
            }
            // The lock button is drawn pressed while the variables are locked.
            let tip = if locked { tl!("Unlock Variables") } else { tl!("Lock Variables") };
            if widgets::icon_button(ui, "lock", tip, locked, 24.0).clicked() {
                app.run("variable.lock", json!({})).ok();
            }
        });
        ui.horizontal(|ui| {
            // Capture records what the art holds now as a new row (paste: take it in); Update
            // writes the art back into the row that is active (save: write it out).
            if widgets::icon_button(ui, "clipboard-paste", tl!("Capture Data Set"), false, 24.0).clicked() {
                app.run("dataset.capture", json!({})).ok();
            }
            let can_update = active.is_some();
            if widgets::icon_button_enabled(ui, "save", tl!("Update Data Set"), false, can_update, 24.0).clicked() {
                app.run("dataset.update", json!({})).ok();
            }
        });
    });
}

/// The panel's (≡) menu: the same commands as its bars and lists, for the ones that act on one
/// row rather than on the selection.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else { return };
    let vars: Vec<String> = st.doc.variables.variables.iter().map(|v| v.name.clone()).collect();
    let sets: Vec<String> = st.doc.variables.datasets.iter().map(|d| d.name.clone()).collect();
    let active = st.doc.variables.active_dataset.clone();
    let bound: Vec<u64> = st.doc.variables.bindings.keys().map(|i| i.0).collect();
    let highlighted = highlighted_row(st);
    let highlighted_bound: Vec<u64> = highlighted.as_ref().map(|h| bound_objects(st, h)).unwrap_or_default();
    let Picked { selected, has_selection, selection_bound, single_type, single_image, selection_graph } = picked(st);
    let locked = st.doc.variables.locked;

    if widgets::menu_item(ui, tl!("New Variable…"), !locked, false) {
        crate::menus::invoke(app, "variable.define", json!({}));
    }
    // Variable Options edits the name and kind of the highlighted variable, else the first.
    if widgets::menu_item(ui, tl!("Variable Options…"), !vars.is_empty() && !locked, false) {
        let name = highlighted.clone().or_else(|| vars.first().cloned()).unwrap_or_default();
        crate::dialogs::variables::open(app, "variable.rename", Some(&name));
    }
    if widgets::menu_item(ui, tl!("Make Text Dynamic"), single_type && !locked, false) {
        app.run("variable.makeTextDynamic", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("Make Visibility Dynamic"), has_selection && !locked, false) {
        app.run("variable.makeVisibilityDynamic", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("Make Image Dynamic"), single_image && !locked, false) {
        app.run("variable.makeImageDynamic", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("Make Graph Dynamic"), selection_graph && !locked, false) {
        app.run("variable.makeGraphDynamic", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("Unbind Variable"), selection_bound, false) {
        app.run("variable.unbind", json!({"ids": selected})).ok();
    }
    if widgets::menu_item(ui, tl!("Delete Variable"), highlighted.is_some() && !locked, false) {
        delete_highlighted(app);
    }
    let lock = if locked { tl!("Unlock Variables") } else { tl!("Lock Variables") };
    if widgets::menu_item(ui, lock, true, false) {
        app.run("variable.lock", json!({})).ok();
    }
    ui.separator();
    if widgets::menu_item(ui, tl!("Capture Data Set"), true, false) {
        app.run("dataset.capture", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("Update Data Set"), active.is_some(), false) {
        app.run("dataset.update", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("New Data Set…"), true, false) {
        crate::menus::invoke(app, "dataset.new", json!({}));
    }
    if widgets::menu_item(ui, tl!("Edit Data Set…"), active.is_some(), false) {
        crate::menus::invoke(app, "dataset.set", json!({}));
    }
    if widgets::menu_item(ui, tl!("Rename Data Set…"), !sets.is_empty(), false) {
        let name = active.clone().or_else(|| sets.first().cloned()).unwrap_or_default();
        crate::dialogs::variables::open(app, "dataset.rename", Some(&name));
    }
    // Only the current data set goes: the rest of the rows stay.
    if widgets::menu_item(ui, tl!("Delete Data Set"), active.is_some(), false) {
        delete_active_dataset(app);
    }
    ui.separator();
    // One file dialog for either kind of data source; the extension picks the reader.
    if widgets::menu_item(ui, tl!("Import…"), true, false) {
        crate::picks::button(app, import_data_source);
    }
    if widgets::menu_item(ui, tl!("Import Data Sets…"), true, false) {
        crate::picks::button(app, import_datasets);
    }
    if widgets::menu_item(ui, tl!("Export Data Sets…"), !vars.is_empty(), false) {
        crate::picks::button(app, export_datasets);
    }
    // Its file dialog, shown off the UI thread, loads the library when it answers.
    if widgets::menu_item(ui, tl!("Load Variable Library…"), true, false) {
        crate::picks::button(app, load_library);
    }
    if widgets::menu_item(ui, tl!("Save Variable Library…"), !vars.is_empty(), false) {
        crate::picks::button(app, save_library);
    }
    ui.separator();
    // What the highlighted variable drives, selected — its bindings are left alone.
    if widgets::menu_item(ui, tl!("Select Bound Object"), !highlighted_bound.is_empty(), false) {
        select_bound_object(app);
    }
    if widgets::menu_item(ui, tl!("Select All Bound Objects"), !bound.is_empty(), false) {
        app.run("select.set", json!({"ids": bound})).ok();
    }
}

/// Variables panel: Import… opens one file dialog for a CSV, TSV, TXT or XML file and runs
/// `variable.import` on the chosen file, which reads it by its extension.
fn import_data_source(app: &mut VectorcraftApp) {
    let pick = crate::FilePick {
        filters: vec![
            crate::FilePick::filter("Data sources", &["csv", "tsv", "txt", "xml"]),
            crate::FilePick::filter("CSV", &["csv", "tsv", "txt"]),
            crate::FilePick::filter("XML", &["xml"]),
        ],
        ..Default::default()
    };
    let Some(path) = crate::picks::open(app, &pick) else { return };
    if let Err(e) = app.run("variable.import", json!({"path": path})) {
        app.status(e);
    }
}

/// Variables panel: Import Data Sets… opens a file dialog for a comma- or tab-separated file and runs `dataset.import` on the chosen file.
fn import_datasets(app: &mut VectorcraftApp) {
    let pick = crate::FilePick { filters: vec![crate::FilePick::filter("CSV", &["csv", "tsv", "txt"])], ..Default::default() };
    let Some(path) = crate::picks::open(app, &pick) else { return };
    if let Err(e) = app.run("dataset.import", json!({"path": path})) {
        app.status(e);
    }
}

/// Variables panel: Export Data Sets… — pick a path and run `dataset.export`.
fn export_datasets(app: &mut VectorcraftApp) {
    let pick = crate::FilePick { name: "datasets.csv".into(), filters: vec![crate::FilePick::filter("CSV", &["csv"])], ..Default::default() };
    let Some(path) = crate::picks::save(app, &pick) else { return };
    if let Err(e) = app.run("dataset.export", json!({"path": path})) {
        app.status(e);
    }
}

/// Variables panel: Load Variable Library… opens a file dialog for an XML file and runs
/// `variable.loadLibrary` on the chosen file.
fn load_library(app: &mut VectorcraftApp) {
    let pick = crate::FilePick { filters: vec![crate::FilePick::filter("XML", &["xml"])], ..Default::default() };
    let Some(path) = crate::picks::open(app, &pick) else { return };
    if let Err(e) = app.run("variable.loadLibrary", json!({"path": path})) {
        app.status(e);
    }
}

/// Variables panel: Save Variable Library… opens a save dialog for an XML file and runs
/// `variable.saveLibrary` with the chosen path.
fn save_library(app: &mut VectorcraftApp) {
    let pick = crate::FilePick { name: "variables.xml".into(), filters: vec![crate::FilePick::filter("XML", &["xml"])], ..Default::default() };
    let Some(path) = crate::picks::save(app, &pick) else { return };
    if let Err(e) = app.run("variable.saveLibrary", json!({"path": path})) {
        app.status(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_data() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        let t = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Alice"})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
        app.session.execute("variable.bind", &json!({"variable": "Name", "ids": [t]})).unwrap();
        app.session.execute("dataset.capture", &json!({})).unwrap();
        app
    }

    #[test]
    fn draws_headless() {
        let mut app = app_with_data();
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
        out.textures_delta.clear();
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("Name") && text.contains("Data Sets") && text.contains("Data Set 1"), "{text}");
    }

    /// Frames of the panel with input events, for typing into its Data Set box.
    struct Driver {
        ctx: egui::Context,
        time: f64,
    }

    impl Driver {
        fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            crate::theme::install_fonts(&ctx);
            Self { ctx, time: 0.0 }
        }

        fn frame(&mut self, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
            self.time += 0.1;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 600.0));
            let input = egui::RawInput { events, time: Some(self.time), screen_rect: Some(screen), ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| show(app, ui));
            out.textures_delta.clear();
            out
        }

        /// The center of the panel's one text box, the Data Set box.
        fn data_set_box(&mut self, app: &mut VectorcraftApp) -> egui::Pos2 {
            use egui::accesskit::Role;
            self.frame(app, vec![]);
            let out = self.frame(app, vec![]);
            let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
            let (_, node) = nodes.iter().find(|(_, n)| matches!(n.role(), Role::TextInput | Role::MultilineTextInput)).expect("the Data Set box");
            let b = node.bounds().expect("bounds");
            egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5)
        }

        /// Clicks the Data Set box, replaces its text with `text` and presses Enter.
        fn type_name(&mut self, app: &mut VectorcraftApp, text: &str) {
            use egui::{Event, Key, Modifiers, PointerButton};
            let pos = self.data_set_box(app);
            let (button, none) = (PointerButton::Primary, Modifiers::default());
            self.frame(app, vec![Event::PointerMoved(pos), Event::PointerButton { pos, button, pressed: true, modifiers: none }]);
            self.frame(app, vec![Event::PointerButton { pos, button, pressed: false, modifiers: none }]);
            let all = Modifiers { ctrl: true, command: true, ..Default::default() };
            let select_all = Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: all };
            self.frame(app, vec![select_all, Event::Text(text.into())]);
            self.frame(app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: none }]);
            self.frame(app, vec![]);
        }

        /// The panel's accessibility nodes after two frames without input.
        fn nodes(&mut self, app: &mut VectorcraftApp) -> Vec<(egui::accesskit::NodeId, egui::accesskit::Node)> {
            self.frame(app, vec![]);
            let out = self.frame(app, vec![]);
            out.platform_output.accesskit_update.expect("an accessibility tree").nodes
        }

        /// Clicks the center of `node`.
        fn click(&mut self, app: &mut VectorcraftApp, node: &egui::accesskit::Node) {
            use egui::{Event, Modifiers, PointerButton};
            let b = node.bounds().expect("bounds");
            let pos = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
            let (button, none) = (PointerButton::Primary, Modifiers::default());
            self.frame(app, vec![Event::PointerMoved(pos), Event::PointerButton { pos, button, pressed: true, modifiers: none }]);
            self.frame(app, vec![Event::PointerButton { pos, button, pressed: false, modifiers: none }]);
            self.frame(app, vec![]);
        }

        /// Clicks the Data Set list button and returns the rows of the open list: the buttons that
        /// were not in the panel before it opened.
        fn open_data_set_list(&mut self, app: &mut VectorcraftApp) -> Vec<egui::accesskit::Node> {
            use egui::accesskit::Role;
            let before = self.nodes(app);
            let (_, list) =
                before.iter().find(|(_, n)| n.role() == Role::ComboBox && n.label() == Some("Data Sets")).expect("the Data Set list button");
            let list = list.clone();
            self.click(app, &list);
            let after = self.nodes(app);
            after.into_iter().filter(|(id, n)| n.role() == Role::Button && !before.iter().any(|(b, _)| b == id)).map(|(_, n)| n).collect()
        }
    }

    fn set_names(app: &mut VectorcraftApp) -> Vec<String> {
        let sets = app.run("dataset.list", json!({})).unwrap();
        sets["datasets"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap_or_default().to_string()).collect()
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// A name typed in the Data Set box and entered renames the current data set in one undo step;
    /// the open list then marks the renamed data set as the current one, and a click on another
    /// row chooses that data set.
    #[test]
    fn the_data_set_box_renames_the_current_data_set() {
        let mut app = app_with_data();
        app.session.execute("dataset.capture", &json!({})).unwrap();
        app.session.execute("dataset.select", &json!({"name": "Data Set 1"})).unwrap();
        let before = undo_steps(&app);
        let mut d = Driver::new();
        d.type_name(&mut app, "Spring");
        assert_eq!(set_names(&mut app), ["Spring", "Data Set 2"]);
        assert_eq!(app.session.active().unwrap().doc.variables.active_dataset.as_deref(), Some("Spring"));
        assert_eq!(undo_steps(&app), before + 1);
        assert_eq!(app.session.active().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Rename Data Set"));
        // The new name is what the box shows, and what the list offers, marked, with the other row.
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("Spring"), "{text}");
        let rows = d.open_data_set_list(&mut app);
        // The accessibility update lists nodes in hash-map order, so the rows are compared by name.
        let mut marked: Vec<(Option<&str>, bool)> = rows.iter().map(|n| (n.label(), n.toggled() == Some(egui::accesskit::Toggled::True))).collect();
        marked.sort();
        assert_eq!(marked, [(Some("Data Set 2"), false), (Some("Spring"), true)]);
        let other = rows.iter().find(|n| n.label() == Some("Data Set 2")).expect("the Data Set 2 row").clone();
        d.click(&mut app, &other);
        assert_eq!(app.session.active().unwrap().doc.variables.active_dataset.as_deref(), Some("Data Set 2"));
    }

    /// The same name adds no undo step; an empty name or another data set's name is refused with a
    /// status message, and the names stay as they were.
    #[test]
    fn the_data_set_box_refuses_an_empty_or_taken_name() {
        let mut app = app_with_data();
        app.session.execute("dataset.capture", &json!({})).unwrap();
        app.session.execute("dataset.select", &json!({"name": "Data Set 1"})).unwrap();
        let before = undo_steps(&app);
        let mut d = Driver::new();
        d.type_name(&mut app, "Data Set 1");
        assert_eq!(undo_steps(&app), before, "the same name runs nothing");
        assert_eq!(app.ui.status, "");
        for name in ["Data Set 2", "   "] {
            app.ui.status.clear();
            d.type_name(&mut app, name);
            assert_eq!(set_names(&mut app), ["Data Set 1", "Data Set 2"], "{name:?}");
            assert_eq!(undo_steps(&app), before, "{name:?}");
            assert!(!app.ui.status.is_empty(), "{name:?} gives no status message");
        }
        assert!(app.ui.status.contains("newName"), "{}", app.ui.status);
        // Through the helper too: an unchanged or padded name runs nothing.
        rename_active_dataset(&mut app, "Data Set 1", " Data Set 1 ");
        assert_eq!(undo_steps(&app), before);
    }

    /// With no data set the box is disabled, so typing in it renames nothing.
    #[test]
    fn the_data_set_box_is_disabled_without_a_data_set() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        let before = undo_steps(&app);
        let mut d = Driver::new();
        let nodes = d.nodes(&mut app);
        let (_, field) = nodes
            .iter()
            .find(|(_, n)| matches!(n.role(), egui::accesskit::Role::TextInput | egui::accesskit::Role::MultilineTextInput))
            .expect("the Data Set box");
        assert!(field.is_disabled(), "the Data Set box is enabled without a data set");
        d.type_name(&mut app, "Spring");
        assert!(set_names(&mut app).is_empty());
        assert_eq!(undo_steps(&app), before);
        assert_eq!(app.ui.status, "");
    }

    /// Each variable row is a list item, and only the highlighted one is selected.
    #[test]
    fn the_rows_report_the_highlight_as_the_selected_state() {
        use egui::accesskit::Role;
        let mut app = app_with_data();
        app.session.execute("variable.define", &json!({"name": "age", "kind": "visibility"})).unwrap();
        app.session.execute("variable.highlight", &json!({"variable": "Name"})).unwrap();
        let nodes = Driver::new().nodes(&mut app);
        let mut rows: Vec<_> = nodes
            .iter()
            .filter(|(_, n)| n.role() == Role::ListItem)
            .map(|(_, n)| (n.label().map(str::to_string), n.is_selected(), n.toggled()))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(rows, [(Some("Name · Alice".to_string()), Some(true), None), (Some("age".to_string()), Some(false), None)]);
    }

    /// A click on a column header sorts the list by that column, and a second click on it
    /// reverses the order; the rows are drawn in the order `variable.list` reports.
    #[test]
    fn a_header_click_sorts_the_list_and_a_second_click_reverses_it() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_data();
        let t = app.session.execute("text.create", &json!({"x": 10, "y": 80, "text": "Bob"})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("layer.setProps", &json!({"id": t, "name": "Aaron"})).unwrap();
        let alice = app.session.doc().unwrap().doc.variables.objects_of("Name")[0];
        app.session.execute("layer.setProps", &json!({"id": alice.0, "name": "Bella"})).unwrap();
        app.session.execute("variable.define", &json!({"name": "age", "kind": "visibility"})).unwrap();
        app.session.execute("variable.bind", &json!({"variable": "age", "ids": [t]})).unwrap();
        let ctx = egui::Context::default();
        let drawn = |ctx: &egui::Context, app: &mut VectorcraftApp| -> Vec<String> {
            let texts = frame_events(ctx, app, vec![], show);
            let mut rows: Vec<(f32, String)> = ["Name", "age"].iter().map(|n| (text_rect(&texts, n).center().y, n.to_string())).collect();
            rows.sort_by(|a, b| a.0.total_cmp(&b.0));
            rows.into_iter().map(|(_, n)| n).collect()
        };
        let listed = |app: &mut VectorcraftApp| -> Vec<String> {
            let list = app.run("variable.list", json!({})).unwrap();
            list["variables"].as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap().to_string()).collect()
        };
        let sort = |app: &VectorcraftApp| {
            let s = app.session.doc().unwrap().variables_sort;
            (s.by, s.descending)
        };
        assert_eq!(drawn(&ctx, &mut app), ["Name", "age"], "the order they were defined in");
        let steps = app.session.doc().unwrap().history.undo.len();

        let header = |ctx: &egui::Context, app: &mut VectorcraftApp, label: &str| text_rect(&frame_events(ctx, app, vec![], show), label).center();
        let at = header(&ctx, &mut app, "Variables");
        click(&ctx, &mut app, at, show);
        assert_eq!(sort(&app), (VariableSortKey::Name, false));
        assert_eq!(drawn(&ctx, &mut app), ["age", "Name"]);
        assert_eq!(listed(&mut app), ["age", "Name"]);
        let at = header(&ctx, &mut app, "Variables");
        click(&ctx, &mut app, at, show);
        assert_eq!(sort(&app), (VariableSortKey::Name, true));
        assert_eq!(drawn(&ctx, &mut app), ["Name", "age"]);

        // Objects: Aaron (age's type) before Bella (Name's), then the reverse.
        let at = header(&ctx, &mut app, "Objects");
        click(&ctx, &mut app, at, show);
        assert_eq!(sort(&app), (VariableSortKey::Object, false));
        assert_eq!(drawn(&ctx, &mut app), ["age", "Name"]);
        let at = header(&ctx, &mut app, "Objects");
        click(&ctx, &mut app, at, show);
        assert_eq!(drawn(&ctx, &mut app), ["Name", "age"]);

        // The type column's header sits left of the Variables header: text before visibility.
        let left = text_rect(&frame_events(&ctx, &mut app, vec![], show), "Variables").left() - 4.0 - TYPE_COLUMN / 2.0;
        let y = header(&ctx, &mut app, "Variables").y;
        click(&ctx, &mut app, egui::pos2(left, y), show);
        assert_eq!(sort(&app), (VariableSortKey::Type, false));
        assert_eq!(drawn(&ctx, &mut app), ["Name", "age"]);
        click(&ctx, &mut app, egui::pos2(left, y), show);
        assert_eq!(sort(&app), (VariableSortKey::Type, true));
        assert_eq!(drawn(&ctx, &mut app), ["age", "Name"]);
        // Sorting is not an edit.
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps, "no undo step");
    }

    #[test]
    fn a_row_the_art_left_behind_is_in_italics() {
        let mut app = app_with_data();
        let t = app.session.doc().unwrap().doc.variables.objects_of("Name")[0];
        // The document no longer matches the row once the art is edited.
        app.session.execute("text.editRange", &json!({"id": t.0, "insert": "Bob"})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("Data Set 1"), "{text}");
        // …and the row's drift is what `dataset.list` reports, so an agent sees it too.
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"][0]["matches"], json!(false));
        // Updating writes the art back into the row.
        app.run("dataset.update", json!({})).unwrap();
        assert_eq!(app.run("dataset.list", json!({})).unwrap()["datasets"][0]["matches"], json!(true));
    }

    /// Choosing a data set, or stepping to one with the arrows, shows a status message when no
    /// object is bound to a variable it has a value for, for example after Import Data Sets…
    /// defines new variables. With such an object bound, choosing a data set or stepping to one
    /// clears the message.
    #[test]
    fn a_data_set_with_nothing_bound_shows_a_status_message() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        let t = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "x"})).unwrap()["id"].as_u64().unwrap();
        app.run("dataset.import", json!({"csv": "Name\nAnn\nBob\n"})).unwrap();
        let (a, b) = ("Data Set 1", "Data Set 2");
        let message = |row: &str| format!("nothing in data set {row} was applied: no object is bound to a variable it has a value for");
        app.run("dataset.select", json!({"name": a})).unwrap();
        assert_eq!(app.ui.status, message(a));
        app.run("dataset.next", json!({})).unwrap();
        assert_eq!(app.ui.status, message(b));
        app.run("dataset.prev", json!({})).unwrap();
        assert_eq!(app.ui.status, message(a));

        app.run("variable.bind", json!({"variable": "Name", "ids": [t]})).unwrap();
        app.run("dataset.select", json!({"name": b})).unwrap();
        assert_eq!(app.ui.status, "");
        // Stepping to a data set that applies clears the message as well.
        for step in ["dataset.next", "dataset.prev"] {
            app.run("variable.unbind", json!({"variable": "Name", "ids": [t]})).unwrap();
            app.run("dataset.select", json!({"name": a})).unwrap();
            assert_eq!(app.ui.status, message(a));
            app.run("variable.bind", json!({"variable": "Name", "ids": [t]})).unwrap();
            app.run(step, json!({})).unwrap();
            assert_eq!(app.ui.status, "", "{step}");
        }
    }

    #[test]
    fn the_panel_menu_lists_the_row_commands() {
        let mut app = app_with_data();
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| menu(&mut app, ui));
        out.textures_delta.clear();
        let text = crate::tests_labels::painted_text(&mut app, menu);
        for item in [
            "New Variable…",
            "Variable Options…",
            "Capture Data Set",
            "Update Data Set",
            "Rename Data Set…",
            "Delete Data Set",
            "Import…",
            "Import Data Sets…",
            "Load Variable Library…",
            "Save Variable Library…",
            "Select Bound Object",
            "Select All Bound Objects",
        ] {
            assert!(text.contains(item), "{item} missing from the panel menu:\n{text}");
        }
    }

    /// The panel menu offers Lock Variables, and Unlock Variables once they are locked.
    #[test]
    fn the_panel_menu_locks_and_unlocks_the_variables() {
        let mut app = app_with_data();
        let text = crate::tests_labels::painted_text(&mut app, menu);
        assert!(text.contains("Lock Variables") && !text.contains("Unlock Variables"), "{text}");
        app.run("variable.lock", json!({})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, menu);
        assert!(text.contains("Unlock Variables"), "{text}");
        // The panel still draws while they are locked.
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("Name") && text.contains("Data Set 1"), "{text}");
    }

    /// Load Variable Library defines a library's variables of every kind and gives no warning. The
    /// warning an SVG's variable data gives about variables that aren't text is translated in every
    /// language but English.
    #[test]
    fn the_svg_variable_data_warning_is_translated() {
        use crate::i18n::{Lang, message};
        let mut app = app_with_data();
        let status = app.ui.status.clone();
        let xml = r#"<variableSets><variableSet><variables><variable varName="Logo" trait="other"/></variables></variableSet></variableSets>"#;
        app.run("variable.loadLibrary", json!({"xml": xml})).unwrap();
        assert_eq!(app.ui.status, status, "the load gives no warning");
        let warning = "variables that aren't text were left out: other (1)";
        let langs: Vec<Lang> = Lang::all().filter(|l| l.code() != "en").collect();
        assert!(langs.len() >= 23, "{} languages", langs.len());
        for l in langs {
            let shown = message(l, warning);
            assert!(!shown.contains("left out") && shown.contains("other") && shown.contains('1'), "{}: {shown}", l.code());
        }
    }

    /// Export Data Sets…'s warnings about variables a new document doesn't read back with their
    /// kind and about variables that make it refuse the file show in the status bar, and so does
    /// its refusal of a header that is another variable's name; all three are translated in every
    /// language but English.
    #[test]
    fn the_export_data_sets_warning_shows_in_the_status_bar() {
        use crate::i18n::{Lang, message};
        let mut app = app_with_data();
        let dir = std::env::temp_dir().join(format!("vc-export-warning-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sets.csv");
        app.run("variable.define", json!({"name": "#Tag", "kind": "text"})).unwrap();
        app.run("dataset.export", json!({"path": path})).unwrap();
        assert_eq!(app.ui.status, "a new document won't read these variables back with their kind: `#Tag`");
        let warning = app.ui.status.clone();
        app.run("variable.define", json!({"name": "%Rate", "kind": "text"})).unwrap();
        app.run("dataset.export", json!({"path": path})).unwrap();
        assert_eq!(app.ui.status, "a new document refuses this file because of these variables: `%Rate`");
        let refuses = app.ui.status.clone();
        app.run("variable.delete", json!({"names": ["%Rate"]})).unwrap();
        app.run("variable.define", json!({"name": "Tag", "kind": "visibility"})).unwrap();
        assert!(app.run("dataset.export", json!({"path": path})).is_err());
        let refused = app.ui.status.clone();
        assert!(refused.contains("would read back as the variable `#Tag`"), "{refused}");
        let _ = std::fs::remove_dir_all(&dir);
        let langs: Vec<Lang> = Lang::all().filter(|l| l.code() != "en").collect();
        assert!(langs.len() >= 23, "{} languages", langs.len());
        for l in langs {
            let shown = message(l, &warning);
            assert!(!shown.contains("won't read") && shown.contains("`#Tag`"), "{}: {shown}", l.code());
            let shown = message(l, &refuses);
            assert!(!shown.contains("refuses") && shown.contains("`%Rate`"), "{}: {shown}", l.code());
            let shown = message(l, &refused);
            assert!(!shown.contains("would read back") && shown.contains("`#Tag`") && shown.contains("`Tag`"), "{}: {shown}", l.code());
        }
    }

    /// The notes of a `.ai` whose variable library was read in part, or couldn't be read, are
    /// translated in every language that translated the note they replace.
    #[test]
    fn the_notes_of_an_ai_variable_library_are_translated() {
        use crate::i18n::{Lang, message};
        let partly = "the file's variable library was partly read (entries skipped: 7): the Variables panel lists the rest";
        let unread = "the file's variable library couldn't be read: the Variables panel doesn't list it";
        let codes = [
            "bg", "bs", "ca", "cs", "da", "de", "el", "es", "fi", "fr", "hu", "id", "it", "ja", "ka", "ko", "nl", "pl", "pt-br", "ru", "sv", "tr",
            "uk", "vi", "zh-hans", "zh-hant",
        ];
        for l in codes.iter().map(|c| Lang::from_code(c).expect("registered")) {
            let shown = message(l, partly);
            assert!(shown != partly && shown.contains('7'), "{}: {shown}", l.code());
            assert_ne!(message(l, unread), unread, "{}", l.code());
        }
    }

    /// What the panel acts on is the highlight, and only while it names a variable: a stale one
    /// (deleted elsewhere, undone) highlights nothing, so Delete and Options… stay disabled.
    #[test]
    fn a_highlight_of_a_deleted_variable_highlights_nothing() {
        let mut app = app_with_data();
        app.session.execute("variable.highlight", &json!({"variable": "Name"})).unwrap();
        assert_eq!(highlighted_row(app.session.doc().unwrap()).as_deref(), Some("Name"));
        assert_eq!(bound_objects(app.session.doc().unwrap(), "Name").len(), 1);
        app.session.execute("variable.delete", &json!({"names": ["Name"]})).unwrap();
        assert!(highlighted_row(app.session.doc().unwrap()).is_none());
    }

    /// The reviewer's regression case, through the calls the panel and its menu make: highlight
    /// a variable, Select Bound Object, then delete one variable and the current dataset. The
    /// selection lands on what the row drives with the binding intact, and the others remain.
    #[test]
    fn selecting_and_deleting_act_on_the_highlighted_row_only() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        let a = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Alice"})).unwrap()["id"].as_u64().unwrap();
        let b = app.session.execute("text.create", &json!({"x": 10, "y": 80, "text": "Bob"})).unwrap()["id"].as_u64().unwrap();
        for (name, id) in [("Name", a), ("Other", b)] {
            app.session.execute("variable.define", &json!({"name": name, "kind": "text"})).unwrap();
            app.session.execute("variable.bind", &json!({"variable": name, "ids": [id]})).unwrap();
        }
        app.session.execute("dataset.capture", &json!({})).unwrap();
        app.session.execute("dataset.capture", &json!({})).unwrap();
        app.session.execute("dataset.select", &json!({"name": "Data Set 1"})).unwrap();

        // A row click highlights; Select Bound Object selects what it drives, binding intact.
        click_row(&mut app, "Name", &[a]);
        assert_eq!(app.session.doc().unwrap().selection.objects, vec![NodeId(a)]);
        assert!(!app.session.doc().unwrap().doc.variables.objects_of("Name").is_empty());
        select_bound_object(&mut app);
        assert_eq!(app.session.doc().unwrap().selection.objects, vec![NodeId(a)]);
        assert!(!app.session.doc().unwrap().doc.variables.objects_of("Name").is_empty());

        // Delete Variable deletes the highlighted one; Delete Data Set the current one.
        delete_highlighted(&mut app);
        delete_active_dataset(&mut app);
        let vars = app.run("variable.list", json!({})).unwrap();
        assert_eq!(vars["variables"].as_array().unwrap().len(), 1);
        assert_eq!(vars["variables"][0]["name"], "Other");
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"].as_array().unwrap().len(), 1);
        assert_eq!(sets["datasets"][0]["name"], "Data Set 2");

        // The panel and its menu still draw with the highlight gone stale.
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("Other") && text.contains("Data Set 2"), "{text}");
        let text = crate::tests_labels::painted_text(&mut app, menu);
        assert!(text.contains("Select Bound Object"), "{text}");
    }

    #[test]
    fn the_panel_is_registered_with_its_own_icon() {
        let row = crate::state::ICON_PANELS.iter().find(|(id, ..)| *id == ID).expect("in the icon column");
        assert_eq!(row.1, "Variables");
        assert_ne!(row.2, "dc-list-view", "Document Info's icon, which the panel used to share");
        assert!(crate::icons::exists(row.2), "{} is not a bundled icon", row.2);
    }

    /// The collapsed column draws `ICON_PANEL_GROUPS`, not `ICON_PANELS`: a panel registered
    /// but in no group opens from the Window menu and then has nowhere to collapse to. Both
    /// lists need an entry, and this is the one that was missing.
    #[test]
    fn the_panel_reaches_the_collapsed_icon_column() {
        let in_column = crate::state::ICON_PANEL_GROUPS.iter().any(|g| g.contains(&ID));
        assert!(in_column, "{ID} is in ICON_PANELS but in no ICON_PANEL_GROUPS row, so the collapsed column never draws it");
        // Every id in a group names a panel that is registered with a label and an icon.
        for group in crate::state::ICON_PANEL_GROUPS {
            for id in *group {
                let row = crate::state::ICON_PANELS.iter().find(|p| p.0 == *id).unwrap_or_else(|| panic!("{id} is in a group but not registered"));
                assert!(!row.1.is_empty() && crate::icons::exists(row.2), "{id}: {} / {}", row.1, row.2);
            }
        }
    }

    /// Window › Variables opens the panel, as Window's other panel items do; its commands are in
    /// the panel, its menu and the command palette.
    #[test]
    fn window_variables_opens_the_panel() {
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let entries = crate::menus::menu_entries(&app);
        let panel = entries
            .iter()
            .find(|e| e.command.as_deref() == Some("window.panel") && e.params.get("panel").and_then(serde_json::Value::as_str) == Some(ID))
            .expect("the Variables panel is listed");
        assert_eq!((panel.path.clone(), panel.label.as_str()), (vec!["Window".to_string()], "Variables"));
        assert!(!entries.iter().any(|e| e.path == ["Window", "Variables"]), "no Variables submenu");
    }

    /// A menu item that needs a name opens the dialog rather than running the command with
    /// empty params, which would only say "missing `name`".
    #[test]
    fn the_menu_dialogs_open_instead_of_failing() {
        let mut app = app_with_data();
        for (cmd, field) in [
            ("variable.define", "kind"),
            ("variable.rename", "newName"),
            ("variable.bind", "variable"),
            ("dataset.new", "name"),
            ("dataset.rename", "newName"),
            ("dataset.select", "name"),
        ] {
            app.ui.dialog = None;
            crate::menus::invoke(&mut app, cmd, json!({}));
            let d = app.ui.dialog.as_ref().unwrap_or_else(|| panic!("{cmd} opened no dialog"));
            assert_eq!(d.kind, crate::dialogs::variables::KIND, "{cmd}");
            assert_eq!(d.str("__command"), cmd);
            assert!(d.fields.contains_key(field), "{cmd}: no `{field}` field");
        }
    }
}
