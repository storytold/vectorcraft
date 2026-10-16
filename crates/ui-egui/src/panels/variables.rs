//! Variables panel: the document's variables (what they drive, which objects) and its
//! datasets. Clicking a dataset applies it; clicking a variable selects what it binds; the
//! arrows step to the next or previous one. Everything the reference panel does here runs a
//! `variable.*` / `dataset.*` command, so the panel and an agent drive the document the same
//! way — the commands are also in Window › Variables, the palette and over MCP.
//!
//! A row's name goes in italics when an edit has moved the art away from what the row says:
//! the document then no longer matches the active dataset, and "Update Data Set" writes the
//! art back into it.

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind, VariableKind};
use vectorcraft_engine::DocState;

use crate::VectorcraftApp;
use crate::widgets;

/// The panel id, as the dock, the Window menu and `window.panel` name it.
pub const ID: &str = "variables";

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

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        widgets::dim_label(ui, tl!("No document"));
        return;
    };
    // Owned snapshots: the rows below run commands, which borrow the app mutably.
    let xml_id = app.session.prefs.identify_objects_by == "xmlId";
    let vars: Vec<VarRow> = st
        .doc
        .variables
        .variables
        .iter()
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

    // The current data set across the top: its name, and the arrows that step through them.
    ui.horizontal(|ui| {
        widgets::field_label(ui, tl!("Data Set"));
        let names: Vec<&str> = sets.iter().map(|s| s.name.as_str()).collect();
        let current = active.map(|s| s.name.as_str()).unwrap_or("");
        let picked = widgets::dropdown_names(ui, "variables-active", current, &names, 160.0);
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

    widgets::subheader(ui, tl!("Variables"));
    if vars.is_empty() {
        widgets::dim_label(ui, tl!("None"));
    }
    for row in &vars {
        ui.horizontal(|ui| {
            // The kind's own icon, so a text variable and a visibility one read apart.
            let icon = match row.kind {
                VariableKind::Text => "type",
                VariableKind::Visibility => "eye",
                VariableKind::Image => "image",
                VariableKind::Graph => "chart-column",
            };
            ui.add(egui::Image::new(crate::icons::source(icon)).fit_to_exact_size(egui::vec2(14.0, 14.0)));
            // The object a variable drives is named beside it, as the reference panel does.
            let text = if row.object.is_empty() { row.name.clone() } else { format!("{} · {}", row.name, row.object) };
            let r = ui.selectable_label(highlighted.as_deref() == Some(row.name.as_str()), text);
            // A click highlights the row and selects what it drives; a double click opens
            // Variable Options for it.
            if r.clicked() {
                click_row(app, &row.name, &row.bound);
            }
            if r.double_clicked() {
                crate::dialogs::variables::open(app, "variable.rename", Some(&row.name));
            }
        });
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

    // The bottom bar: make the selection dynamic, and define and delete what the lists show.
    ui.separator();
    ui.horizontal(|ui| {
        if widgets::icon_button_enabled(ui, "type", tl!("Make Text Dynamic"), false, single_type, 24.0).clicked() {
            app.run("variable.makeTextDynamic", json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "eye", tl!("Make Visibility Dynamic"), false, has_selection, 24.0).clicked() {
            app.run("variable.makeVisibilityDynamic", json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "image", tl!("Make Image Dynamic"), false, single_image, 24.0).clicked() {
            app.run("variable.makeImageDynamic", json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "chart-column", tl!("Make Graph Dynamic"), false, selection_graph, 24.0).clicked() {
            app.run("variable.makeGraphDynamic", json!({})).ok();
        }
        if widgets::icon_button(ui, "plus", tl!("New Variable…"), false, 24.0).clicked() {
            crate::menus::invoke(app, "variable.define", json!({}));
        }
        if widgets::icon_button_enabled(ui, "link-2-off", tl!("Unbind Variable"), false, selection_bound, 24.0).clicked() {
            app.run("variable.unbind", json!({"ids": selected})).ok();
        }
        // The bin deletes the highlighted variable only, as the reference panel's does.
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Variable"), false, highlighted.is_some(), 24.0).clicked() {
            delete_highlighted(app);
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

    if widgets::menu_item(ui, tl!("New Variable…"), true, false) {
        crate::menus::invoke(app, "variable.define", json!({}));
    }
    // Variable Options renames the highlighted variable, else the first.
    if widgets::menu_item(ui, tl!("Variable Options…"), !vars.is_empty(), false) {
        let name = highlighted.clone().or_else(|| vars.first().cloned()).unwrap_or_default();
        crate::dialogs::variables::open(app, "variable.rename", Some(&name));
    }
    if widgets::menu_item(ui, tl!("Make Text Dynamic"), single_type, false) {
        app.run("variable.makeTextDynamic", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("Make Visibility Dynamic"), has_selection, false) {
        app.run("variable.makeVisibilityDynamic", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("Make Image Dynamic"), single_image, false) {
        app.run("variable.makeImageDynamic", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("Make Graph Dynamic"), selection_graph, false) {
        app.run("variable.makeGraphDynamic", json!({})).ok();
    }
    if widgets::menu_item(ui, tl!("Unbind Variable"), selection_bound, false) {
        app.run("variable.unbind", json!({"ids": selected})).ok();
    }
    if widgets::menu_item(ui, tl!("Delete Variable"), highlighted.is_some(), false) {
        delete_highlighted(app);
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
    if widgets::menu_item(ui, tl!("Import Data Sets…"), true, false) {
        import_datasets(app);
    }
    if widgets::menu_item(ui, tl!("Export Data Sets…"), !vars.is_empty(), false) {
        export_datasets(app);
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

/// Variables panel: Import Data Sets… — pick a CSV and run `dataset.import`.
fn import_datasets(app: &mut VectorcraftApp) {
    let pick = crate::FilePick { filters: vec![("CSV", &["csv", "txt"])], ..Default::default() };
    let Some(path) = crate::picks::open(app, &pick) else { return };
    if let Err(e) = app.run("dataset.import", json!({"path": path})) {
        app.status(e);
    }
}

/// Variables panel: Export Data Sets… — pick a path and run `dataset.export`.
fn export_datasets(app: &mut VectorcraftApp) {
    let pick = crate::FilePick { name: "datasets.csv".into(), filters: vec![("CSV", &["csv"])], ..Default::default() };
    let Some(path) = crate::picks::save(app, &pick) else { return };
    if let Err(e) = app.run("dataset.export", json!({"path": path})) {
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
            "Select Bound Object",
            "Select All Bound Objects",
        ] {
            assert!(text.contains(item), "{item} missing from the panel menu:\n{text}");
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
