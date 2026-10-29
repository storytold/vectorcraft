use super::dnd::{group_of, selection};
use super::libraries::SwatchLibraries;
use super::{Kind, LibraryKind, Ui, Value, VectorcraftApp, View, active_paint, json, library_panel, menu_item, pstate, set_pstate, widgets};

/// The selected swatches and groups that can be deleted (all but None and Registration).
pub(super) fn deletable(sel: &[String]) -> Vec<String> {
    sel.iter().filter(|n| !n.starts_with('[')).cloned().collect()
}

/// The selected swatch when exactly one swatch with an editor is selected (not None, Registration
/// or a group).
fn editable(app: &VectorcraftApp, sel: &[String]) -> Option<String> {
    match sel {
        [n] if !n.starts_with('[') && app.session.active().is_some_and(|st| st.doc.swatch(n).is_some()) => Some(n.clone()),
        _ => None,
    }
}

/// The selected solid-colour swatches in selection order (Merge Swatches keeps the first).
pub(super) fn mergeable(app: &VectorcraftApp, sel: &[String]) -> Vec<String> {
    let Some(st) = app.session.active() else { return vec![] };
    sel.iter().filter(|n| st.doc.swatch(n).is_some_and(|w| w.paint.color().is_some())).cloned().collect()
}

/// The first selected colour group (Ungroup Color Group).
pub(super) fn selected_group(app: &VectorcraftApp, sel: &[String]) -> Option<String> {
    let d = &app.session.active()?.doc;
    sel.iter().find(|n| d.swatch_groups.iter().any(|g| g.name == **n)).cloned()
}

/// Edit or Apply Color Group: Recolor Artwork on colour group `group` (and the selected art).
pub(super) fn edit_group(app: &mut VectorcraftApp, group: &str) {
    app.run("ui.recolorDialog", json!({ "group": group })).ok();
}

/// Delete `names` (swatches and colour groups) after asking, or at once with `now` (Alt-click).
pub(super) fn delete(app: &mut VectorcraftApp, names: Vec<String>, now: bool) {
    let params = json!({"names": names});
    if now {
        app.run("swatch.delete", params).ok();
        return;
    }
    let message = match names.as_slice() {
        [n] => crate::i18n::fmt(tl!("Delete “{name}”?"), &[("name", n)]),
        _ => crate::i18n::fmt(tl!("Delete these {count} swatches and groups?"), &[("count", &names.len().to_string())]),
    };
    crate::dialogs::confirm::ask(app, &message, tl!("Art using a deleted global swatch keeps its colour."), "swatch.delete", params);
}

pub(super) fn bottom(app: &mut VectorcraftApp, ui: &mut Ui, sel: &[String]) {
    let kind: Kind = pstate(ui.ctx(), "swatch-kind");
    widgets::bottom_bar(ui, |ui| {
        let open = library_panel::any_open::<SwatchLibraries>(&app.ui);
        let lr = widgets::icon_button(ui, "library", SwatchLibraries::MENU, open, 24.0);
        egui::Popup::menu(&lr).show(|ui| {
            ui.set_min_width(200.0);
            library_panel::library_menu::<SwatchLibraries>(app, ui);
        });
        let kr = widgets::icon_button(ui, "dc-swatch-kinds", tl!("Show Swatch Kinds"), kind != Kind::All, 24.0);
        egui::Popup::menu(&kr).show(|ui| {
            for (k, label) in Kind::ALL {
                if menu_item(ui, label, true, k == kind) {
                    set_pstate(ui.ctx(), "swatch-kind", k);
                }
            }
        });
        // With a colour group selected the options button edits or applies the group.
        let opts = editable(app, sel);
        if let Some(g) = selected_group(app, sel).filter(|_| opts.is_none()) {
            if widgets::icon_button(ui, "palette", tl!("Edit or Apply Color Group"), false, 24.0).clicked() {
                edit_group(app, &g);
            }
        } else if widgets::icon_button_enabled(ui, "dc-options", tl!("Swatch Options"), false, opts.is_some(), 24.0).clicked()
            && let Some(n) = opts
        {
            app.run("ui.swatchOptions", json!({"name": n})).ok();
        }
        ui.add_space((ui.available_width() - 3.0 * 28.0).max(0.0));
        if widgets::icon_button(ui, "dc-folder", tl!("New Color Group"), false, 24.0).clicked() {
            app.run("ui.newColorGroup", json!({"swatches": deletable(sel)})).ok();
        }
        // Alt-click skips the dialog; Ctrl/Cmd-click makes a spot color, and Ctrl/Cmd+Shift-click
        // makes a global process color.
        if widgets::icon_button(ui, "dc-new-item", tl!("New Swatch"), false, 24.0).clicked() {
            new_swatch(app, sel, ui.input(|i| i.modifiers));
        }
        // Alt-click deletes without asking.
        let del = deletable(sel);
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Swatch"), false, !del.is_empty(), 24.0).clicked() {
            delete(app, del, ui.input(|i| i.modifiers.alt));
        }
    });
}

/// The colour group the selection points at: the last selected group, or the group of the last
/// selected swatch.
fn target_group(app: &VectorcraftApp, sel: &[String]) -> Option<String> {
    group_of(&app.session.active()?.doc, sel.last()?)
}

/// New Swatch from the active paint (a color goes into the selected color group), for a click
/// with modifiers `m`: a plain click opens the dialog and an Alt-click saves at once. Cmd/Ctrl
/// makes a color a spot color, and Cmd/Ctrl+Shift makes it a global process color.
pub(super) fn new_swatch(app: &mut VectorcraftApp, sel: &[String], m: egui::Modifiers) {
    let paint = active_paint(app);
    let group = paint.color().and_then(|_| target_group(app, sel));
    let (spot, global) = (m.command && !m.shift, m.command && m.shift);
    if !m.alt {
        app.run("ui.newSwatch", json!({"spot": spot, "global": global, "group": group})).ok();
        return;
    }
    let mut params = crate::panels::swatch_paint_params(&paint);
    if paint.color().is_some() {
        params["spot"] = json!(spot);
        params["global"] = json!(global);
    }
    if let Some(g) = group {
        params["group"] = json!(g);
    }
    app.run("swatch.new", params).ok();
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let view: View = pstate(ui.ctx(), "swatch-view");
    let selected = selection(app, ui);
    let del = deletable(&selected);
    let opts = editable(app, &selected);
    if menu_item(ui, tl!("New Swatch…"), true, false) {
        new_swatch(app, &selected, egui::Modifiers::NONE);
    }
    if menu_item(ui, tl!("New Color Group…"), true, false) {
        app.run("ui.newColorGroup", json!({"swatches": del})).ok();
    }
    if menu_item(ui, tl!("Duplicate Swatch"), opts.is_some(), false)
        && let Some(n) = &opts
    {
        app.run("swatch.duplicate", json!({"name": n})).ok();
    }
    // The first selected colour is kept.
    let merge = mergeable(app, &selected);
    if menu_item(ui, tl!("Merge Swatches"), merge.len() > 1, false) {
        app.run("swatch.merge", json!({ "names": merge })).ok();
    }
    if menu_item(ui, tl!("Delete Swatch"), !del.is_empty(), false) {
        delete(app, del, false);
    }
    let group = selected_group(app, &selected);
    if menu_item(ui, tl!("Edit or Apply Color Group…"), group.is_some(), false)
        && let Some(g) = &group
    {
        edit_group(app, g);
    }
    if menu_item(ui, tl!("Ungroup Color Group"), group.is_some(), false)
        && let Some(g) = group
    {
        app.run("swatch.ungroup", json!({ "name": g })).ok();
    }
    if menu_item(ui, tl!("Select All Unused"), true, false)
        && let Ok(r) = app.run("swatch.unused", json!({}))
    {
        let names: Vec<String> = r["names"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        set_pstate(ui.ctx(), "swatch-selected", names);
    }
    if menu_item(ui, tl!("Add Used Colors"), true, false) {
        app.run("swatch.addUsedColors", json!({})).ok();
    }
    ui.separator();
    if menu_item(ui, tl!("Sort by Name"), true, false) {
        app.run("swatch.sortByName", json!({})).ok();
    }
    if menu_item(ui, tl!("Sort by Kind"), true, false) {
        app.run("swatch.sortByKind", json!({})).ok();
    }
    ui.separator();
    for (v, label) in View::ALL {
        if menu_item(ui, label, true, v == view) {
            set_pstate(ui.ctx(), "swatch-view", v);
        }
    }
    ui.separator();
    let find: bool = pstate(ui.ctx(), "swatch-show-find");
    if menu_item(ui, tl!("Show Find Field"), true, find) {
        set_pstate(ui.ctx(), "swatch-show-find", !find);
    }
    ui.separator();
    if menu_item(ui, tl!("Swatch Options…"), opts.is_some(), false)
        && let Some(n) = opts
    {
        app.run("ui.swatchOptions", json!({"name": n})).ok();
    }
    if menu_item(ui, tl!("Spot Colors…"), app.session.active().is_some(), false) {
        app.run("ui.spotColors", json!({})).ok();
    }
    ui.separator();
    ui.menu_button(tl!("Open Swatch Library"), |ui| library_panel::library_menu::<SwatchLibraries>(app, ui));
    if menu_item(ui, tl!("Save Swatch Library…"), true, false) {
        app.run("ui.saveSwatchLibrary", json!({ "names": deletable(&selected) })).ok();
    }
}
