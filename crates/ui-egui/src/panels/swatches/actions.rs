use craft::keymap::{CommandRef, Mods};
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use serde_json::{Value, json};

use super::dnd::group_of;
use super::libraries::SwatchLibraries;
use super::library_panel;
use super::{View, active_paint, selected};
use crate::VectorcraftApp;

/// The selected swatches and groups that can be deleted: every color group in the document and
/// every swatch that `Swatch::is_reserved` does not reserve (None and Registration). Names the
/// document does not hold are left out.
pub(super) fn deletable(app: &VectorcraftApp, sel: &[String]) -> Vec<String> {
    let Some(d) = app.session.active().map(|st| &st.doc) else { return vec![] };
    sel.iter().filter(|n| d.swatch(n).map_or_else(|| d.swatch_groups.iter().any(|g| g.name == **n), |w| !w.is_reserved())).cloned().collect()
}

/// The selected swatch when exactly one swatch with an editor is selected (not None, Registration
/// or a group).
pub(super) fn editable(app: &VectorcraftApp, sel: &[String]) -> Option<String> {
    match sel {
        [n] if app.session.active().and_then(|st| st.doc.swatch(n)).is_some_and(|w| !w.is_reserved()) => Some(n.clone()),
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

/// The selected color group when it is the whole selection (Color Group Options).
pub(super) fn sole_group(app: &VectorcraftApp, sel: &[String]) -> Option<String> {
    match sel {
        [_] => selected_group(app, sel),
        _ => None,
    }
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
        _ => crate::i18n::tn(names.len() as u64, "Delete this {n} swatch or group?", "Delete these {n} swatches and groups?"),
    };
    crate::dialogs::confirm::ask(app, &message, tl!("Art using a deleted global swatch keeps its colour."), "swatch.delete", params);
}

/// Duplicates the swatches `names` (`swatch.duplicate`, each copy just after its swatch and in its
/// color group) in one undo step and selects the copies. A failure is shown in the status bar and
/// changes nothing.
pub(super) fn duplicate(app: &mut VectorcraftApp, names: &[String]) {
    let made = if let [name] = names {
        app.run("swatch.duplicate", json!({ "name": name })).map(|r| vec![r])
    } else {
        let commands: Vec<Value> = names.iter().map(|n| json!({ "command": "swatch.duplicate", "params": { "name": n } })).collect();
        app.run("command.batch", json!({ "label": "Duplicate Swatch", "commands": commands }))
            .map(|r| r["results"].as_array().cloned().unwrap_or_default())
    };
    match made {
        Ok(made) => app.ui.swatches_panel.set_swatches(made.iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect()),
        Err(e) => app.status(e),
    }
}

/// The colour group the selection points at: the last selected group, or the group of the last
/// selected swatch.
fn target_group(app: &VectorcraftApp, sel: &[String]) -> Option<String> {
    group_of(&app.session.active()?.doc, sel.last()?)
}

/// New Swatch from the active paint (a color goes into the selected color group), for a click
/// with modifiers `m`: a plain click opens the dialog and an Alt-click saves at once. Cmd/Ctrl
/// makes a color a spot color, and Cmd/Ctrl+Shift makes it a global process color.
pub(super) fn new_swatch(app: &mut VectorcraftApp, sel: &[String], m: Mods) {
    let paint = active_paint(app);
    let group = paint.color().and_then(|_| target_group(app, sel));
    let (spot, global) = (m.cmd && !m.shift, m.cmd && m.shift);
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

/// The id prefix of the panel menu's rows.
const ROW: &str = "ui.swatches";

fn id(row: &str) -> String {
    format!("{ROW}.{row}")
}

fn cmd(row: &str) -> PanelRow {
    PanelRow::Command(CommandRef::new(id(row)))
}

fn view_ref(v: View) -> CommandRef {
    CommandRef::with_params(id("view"), json!({ "view": v }))
}

/// The libraries of Open Swatch Library, each opening in its panel, then Other Library….
fn library_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let mut rows = library_panel::library_rows::<SwatchLibraries>(app);
    rows.push(PanelRow::Command(CommandRef::new("window.swatchLibrary.other")));
    rows
}

/// The rows of the panel menu, in order.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let mut rows = vec![
        cmd("newSwatch"),
        cmd("newColorGroup"),
        cmd("duplicate"),
        cmd("merge"),
        cmd("delete"),
        cmd("editGroup"),
        cmd("ungroup"),
        cmd("groupOptions"),
        cmd("selectUnused"),
        cmd("addUsedColors"),
        cmd("addSelectedColors"),
        PanelRow::Separator,
        cmd("sortByName"),
        cmd("sortByKind"),
        PanelRow::Separator,
    ];
    rows.extend(View::ALL.iter().map(|(v, _)| PanelRow::Command(view_ref(*v))));
    rows.extend([
        PanelRow::Separator,
        cmd("showFind"),
        cmd("showRecent"),
        PanelRow::Separator,
        cmd("options"),
        cmd("spotColors"),
        PanelRow::Separator,
        PanelRow::Submenu(tl!("Open Swatch Library").into(), library_rows(app)),
        PanelRow::Command(CommandRef::with_params("ui.saveSwatchLibrary", json!({ "names": deletable(app, &selected(app)) }))),
    ]);
    rows
}

/// The enabled and checked state of the menu's rows.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let sel = selected(app);
    let (del, opts, group) = (deletable(app, &sel), editable(app, &sel), selected_group(app, &sel));
    let art_selected = app.session.active().is_some_and(|st| !st.selection.objects.is_empty());
    let doc = app.session.active().is_some();
    let enabled = [
        ("duplicate", opts.is_some()),
        ("merge", mergeable(app, &sel).len() > 1),
        ("delete", !del.is_empty()),
        ("editGroup", group.is_some()),
        ("ungroup", group.is_some()),
        ("groupOptions", sole_group(app, &sel).is_some()),
        ("addSelectedColors", art_selected),
        ("options", opts.is_some()),
        ("spotColors", doc),
    ];
    for (row, on) in enabled {
        states.set(CommandRef::new(id(row)), CommandState::new().enabled(on));
    }
    states.set(CommandRef::new(id("showFind")), CommandState::new().checked(app.ui.swatches_panel.show_find));
    states.set(CommandRef::new(id("showRecent")), CommandState::new().checked(app.ui.swatches_show_recent_colors));
    for (v, _) in View::ALL {
        states.set(view_ref(v), CommandState::new().checked(v == app.ui.swatches_view));
    }
}

/// Runs panel menu row `id` (`ui.swatches.<row>`); `None` for another panel's id.
pub(crate) fn run(app: &mut VectorcraftApp, row: &str, params: &Value) -> Option<Result<Value, String>> {
    let row = row.strip_prefix(ROW)?.strip_prefix('.')?;
    let sel = selected(app);
    let (del, opts, group) = (deletable(app, &sel), editable(app, &sel), selected_group(app, &sel));
    let done = Ok(Value::Null);
    Some(match row {
        "newSwatch" => {
            new_swatch(app, &sel, Mods::default());
            done
        }
        "newColorGroup" => app.run("ui.newColorGroup", json!({ "swatches": del })),
        "duplicate" => match opts {
            Some(n) => app.run("swatch.duplicate", json!({ "name": n })),
            None => done,
        },
        "merge" => app.run("swatch.merge", json!({ "names": mergeable(app, &sel) })),
        "delete" => {
            delete(app, del, false);
            done
        }
        "editGroup" => {
            if let Some(g) = group {
                edit_group(app, &g);
            }
            done
        }
        "ungroup" => match group {
            Some(g) => app.run("swatch.ungroup", json!({ "name": g })),
            None => done,
        },
        "groupOptions" => match sole_group(app, &sel) {
            Some(g) => app.run("ui.colorGroupOptions", json!({ "group": g })),
            None => done,
        },
        "selectUnused" => app.run("swatch.unused", json!({})).map(|r| {
            let names = r["names"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            app.ui.swatches_panel.set_swatches(names);
            r
        }),
        "addUsedColors" => app.run("swatch.addUsedColors", json!({})),
        // The selected art's new colors, as global swatches its paints link to.
        "addSelectedColors" => app.run("swatch.addUsedColors", json!({"selection": true, "global": true})),
        "sortByName" => app.run("swatch.sortByName", json!({})),
        "sortByKind" => app.run("swatch.sortByKind", json!({})),
        "view" => app.run("ui.panelView", json!({ "panel": "swatches", "view": params["view"] })),
        "showFind" => {
            let panel = &mut app.ui.swatches_panel;
            panel.show_find = !panel.show_find;
            done
        }
        "showRecent" => {
            app.ui.swatches_show_recent_colors = !app.ui.swatches_show_recent_colors;
            done
        }
        "options" => match opts {
            Some(n) => app.run("ui.swatchOptions", json!({ "name": n })),
            None => done,
        },
        "spotColors" => app.run("ui.spotColors", json!({})),
        _ => return None,
    })
}
