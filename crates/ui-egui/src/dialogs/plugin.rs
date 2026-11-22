//! WebAssembly plug-ins in the menus and their generated dialogs.
//!
//! Object › Plug-ins lists the installed object filters (`plugin.dialog {id}`), then Install
//! Plug-in… and Reload Plug-ins; Effect › Plug-ins lists the live-effect plug-ins, which open the
//! effect dialog (`effect.dialog`). A filter with parameters opens dialog `plugin`: one field per
//! declared parameter (fields named after them; `__plugin` is the plug-in id) with a live
//! Preview, and OK keeps the run as one undo step (`plugin.run`). A filter without parameters runs
//! at once. A heavy filter (its manifest's `budget`) has no Preview, since a run may take minutes:
//! it runs in the background ([`crate::plugin_runs`]). A light filter that reports its progress
//! (plug-in API level 2 with the `progress` permission) and calls no commands runs in the
//! background too, with its progress and Cancel, when it runs at once or when OK runs it without a
//! kept preview ([`in_background`]). A filter or live effect with commands of
//! its own is a submenu named after it, holding its item and then its commands; a command with
//! parameters opens the same dialog for them ([`open_command`]). A command whose manifest names
//! another menu is listed in that menu instead ([`place_commands`]). Exporters are formats of File › Export ›
//! Export As; one with parameters opens dialog `pluginExport` once the file is picked, and its
//! Export button writes the file. Importers open the files File › Open finds no built-in format
//! for, and File › Open With lists them, to open a file with one of them by name; one with
//! parameters opens dialog `pluginImport` once the file is picked, and its Open button opens it.
//!
//! Dialog `pluginInstall` asks a person to approve a plug-in's permissions or heavy budget. What it
//! shows and installs is held by the app, not in its fields; a control request or synthetic input
//! can't confirm it. Enter confirms it only when OK has the keyboard focus. OK takes no click, Enter
//! or Space until the question has been on screen in the focused window for [`APPROVAL_DELAY`], and
//! that time starts over for every new question and whenever the window loses the focus.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::plugin::InstallApproval;
use vectorcraft_plugins::{Budget, ContextMenu, ContextWhen, Kind, Manifest, Plugin, permissions, registry};

use super::{DialogAction, DialogSpec, form};
use crate::VectorcraftApp;
use crate::menus::Item;
use crate::state::Dialog;
use crate::widgets;

/// The dialog kind of plug-in filters.
pub const KIND: &str = "plugin";

const CMD: &str = "plugin.run";

/// Object › Plug-ins' header while no filter plug-in is installed.
pub const NO_FILTERS: &str = "No filter plug-ins installed";

/// Headed by the plug-in's name, as it is (plug-in text is never translated).
pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |d| d.str("__label"), body, confirm, preview: true, max_width: Some(420.0), ..DialogSpec::FORM };

/// A menu label for plug-in `name`: with an ellipsis when it opens a dialog.
fn label(name: &str, has_params: bool) -> &'static str {
    let dots = if has_params && !name.ends_with('…') { "…" } else { "" };
    registry::intern(&format!("{name}{dots}"))
}

/// A filter or live effect `plugin`'s item `own`, as the menu lists it: alone, or for a plug-in
/// with commands of its own, a submenu named after it holding `own` and then its commands. Each
/// command runs its command id (`plugin.<id>.<command>`), with an ellipsis when it has parameters,
/// whose dialog [`open_command`] opens.
fn with_commands(plugin: &Plugin, own: Item) -> Item {
    let m = plugin.manifest();
    if m.commands.is_empty() {
        return own;
    }
    let mut items = vec![own, Item::Sep];
    items.extend(m.commands.iter().map(|c| {
        let id = registry::intern(&vectorcraft_engine::cmd::plugin::command_id(&m.id, &c.id));
        Item::Cmd(label(&c.shown_label(), !c.params.is_empty()), id, Value::Null)
    }));
    Item::Sub(registry::intern(&permissions::display_text(m.name.trim(), permissions::MAX_SHOWN_NAME)), items)
}

/// Moves each plug-in command whose manifest's `menu` names another menu
/// ([`vectorcraft_plugins::MenuPlace`]) from its plug-in's submenu into that menu of `tree`: after
/// the built-in item its `after` names (with or without an ellipsis), or at the end of the menu.
/// Its label ends with the plug-in's name ([`vectorcraft_plugins::Command::menu_label`]).
/// Commands placed at the same spot are listed in plug-in id order and then in manifest order. A
/// plug-in's submenu left with its own item alone becomes that item. A command whose menu `tree`
/// doesn't have, or that [`vectorcraft_engine::cmd::plugin::placed_menu`] leaves in its plug-in's
/// submenu, stays there.
pub fn place_commands(tree: &mut [(&'static str, Vec<Item>)]) {
    let mut spots: Vec<(&'static [&'static str], Option<String>, Vec<Item>)> = Vec::new();
    for p in registry::list() {
        let m = p.manifest();
        if !matches!(m.kind, Kind::Filter | Kind::Effect) {
            continue;
        }
        for c in &m.commands {
            let Some(place) = &c.place else { continue };
            if vectorcraft_engine::cmd::plugin::placed_menu(&p, c).is_none() {
                continue;
            }
            // An `after` that names no built-in item of the menu means the end of it, as no `after` does.
            // The spot is the item's own label, so both labels of an item that switches are one spot.
            let after = place.after.as_deref().and_then(|a| {
                let menu = menu_at(tree, place.path)?;
                menu.iter().find(|it| is_anchor(it, anchor_key(a))).and_then(own_key).map(str::to_string)
            });
            let id = registry::intern(&vectorcraft_engine::cmd::plugin::command_id(&m.id, &c.id));
            let item = Item::Cmd(label(&c.menu_label(&m.name), !c.params.is_empty()), id, Value::Null);
            match spots.iter_mut().find(|s| s.0 == place.path && s.1 == after) {
                Some(s) => s.2.push(item),
                None => spots.push((place.path, after, vec![item])),
            }
        }
    }
    spots.retain(|s| menu_at(tree, s.0).is_some());
    if spots.is_empty() {
        return;
    }
    let placed: std::collections::HashSet<&str> = spots
        .iter()
        .flat_map(|s| s.2.iter())
        .filter_map(|it| match it {
            Item::Cmd(_, id, _) => Some(*id),
            _ => None,
        })
        .collect();
    // Plug-in submenus are under Object › Plug-ins and Effect › Plug-ins.
    for (_, items) in tree.iter_mut().filter(|(title, _)| matches!(*title, "Object" | "Effect")) {
        leave_out(items, &placed);
    }
    for (path, after, items) in spots {
        let Some(menu) = menu_at(tree, path) else { continue };
        let at = after.and_then(|a| menu.iter().position(|it| own_key(it) == Some(a.as_str()))).map_or(menu.len(), |i| i + 1);
        menu.splice(at..at, items);
    }
}

/// A label as `after` matches it: trimmed, without a closing ellipsis.
fn anchor_key(label: &str) -> &str {
    label.trim().trim_end_matches('…').trim_end()
}

/// Is `it` the built-in item or submenu labeled `key` ([`anchor_key`])? An item of
/// [`crate::menus::DYNAMIC_LABELS`] matches by the label of either of its states
/// ([`vectorcraft_engine::cmd::plugin::other_state`]). Items that list a plug-in are never an anchor.
fn is_anchor(it: &Item, key: &str) -> bool {
    let other = |l: &str| crate::menus::DYNAMIC_LABELS.contains(&l) && vectorcraft_engine::cmd::plugin::other_state(l).is_some_and(|o| o == key);
    own_key(it).is_some_and(|own| own == key || matches!(it, Item::Cmd(l, _, _) if other(l)))
}

/// The label of `it` as [`anchor_key`] reads it, when `it` is a built-in item or submenu that can be
/// an anchor; `None` for an item that lists a plug-in.
fn own_key(it: &Item) -> Option<&str> {
    match it {
        Item::Cmd(l, id, p) if !crate::menus::lists_plugin(id, p) => Some(anchor_key(l)),
        Item::Sub(l, children) if !crate::menus::holds_plugin_commands(children) => Some(anchor_key(l)),
        _ => None,
    }
}

/// The built-in menu of `tree` at `path`: a top-level menu, then its submenus by label.
fn menu_at<'a>(tree: &'a mut [(&'static str, Vec<Item>)], path: &[&str]) -> Option<&'a mut Vec<Item>> {
    let (first, rest) = path.split_first()?;
    let mut menu = &mut tree.iter_mut().find(|(title, _)| title == first)?.1;
    for segment in rest {
        menu = menu.iter_mut().find_map(|it| match it {
            Item::Sub(l, children) if l == segment && !crate::menus::holds_plugin_commands(children) => Some(children),
            _ => None,
        })?;
    }
    Some(menu)
}

/// Takes the commands in `placed` out of the plug-in submenus under `items`; a submenu left with no
/// command becomes its plug-in's own item.
fn leave_out(items: &mut [Item], placed: &std::collections::HashSet<&str>) {
    for it in items.iter_mut() {
        let Item::Sub(_, children) = it else { continue };
        if !crate::menus::holds_plugin_commands(children) {
            leave_out(children, placed);
            continue;
        }
        children.retain(|c| !matches!(c, Item::Cmd(_, id, _) if placed.contains(id)));
        if !children.iter().any(|c| matches!(c, Item::Cmd(_, id, _) if vectorcraft_engine::cmd::plugin::is_command_id(id)))
            && let Some(own) = children.first().cloned()
        {
            *it = own;
        }
    }
}

/// Does condition `when` of a context menu item hold for objects `ids` of `doc` (the selection on
/// the canvas, the clicked rows in the Layers panel)? `selection`: one or more; `oneObject`: one,
/// which is not a layer; `typeObject`: one of them is a type object; `layerRow`: each is a layer or
/// sublayer. Without a condition the item can always be chosen.
pub fn context_holds(doc: &vectorcraft_doc::Document, ids: &[u64], when: Option<ContextWhen>) -> bool {
    let nodes = || ids.iter().filter_map(|id| doc.node(vectorcraft_doc::NodeId(*id)));
    match when {
        None => true,
        Some(ContextWhen::Selection) => !ids.is_empty(),
        Some(ContextWhen::OneObject) => ids.len() == 1 && nodes().count() == 1 && nodes().all(|n| !n.is_layer()),
        Some(ContextWhen::TypeObject) => nodes().any(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Text(_))),
        Some(ContextWhen::LayerRow) => !ids.is_empty() && nodes().count() == ids.len() && nodes().all(|n| n.is_layer()),
    }
}

/// One plug-in command of a context menu: its label (with an ellipsis when it has parameters), its
/// command id and whether its condition holds.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextCommand {
    pub label: &'static str,
    pub command: &'static str,
    pub enabled: bool,
}

/// The plug-in commands context menu `menu` lists for objects `ids` of `doc`
/// ([`vectorcraft_plugins::Command::context`]): for each filter or live effect, in plug-in id
/// order, its name as it is and its commands in manifest order, each enabled only while its `when`
/// holds ([`context_holds`]).
pub fn context_commands(menu: ContextMenu, doc: &vectorcraft_doc::Document, ids: &[u64]) -> Vec<(&'static str, Vec<ContextCommand>)> {
    registry::list()
        .iter()
        .filter(|p| matches!(p.manifest().kind, Kind::Filter | Kind::Effect))
        .filter_map(|p| {
            let m = p.manifest();
            let commands: Vec<ContextCommand> = m
                .context_commands(menu)
                .map(|(c, when)| ContextCommand {
                    label: label(&c.shown_label(), !c.params.is_empty()),
                    command: registry::intern(&vectorcraft_engine::cmd::plugin::command_id(&m.id, &c.id)),
                    enabled: context_holds(doc, ids, when),
                })
                .collect();
            let name = registry::intern(&permissions::display_text(m.name.trim(), permissions::MAX_SHOWN_NAME));
            (!commands.is_empty()).then_some((name, commands))
        })
        .collect()
}

/// The canvas context menu's Plug-ins submenu for the selection `ids`: a submenu per plug-in that
/// lists commands there, named after it, each item running its command on `ids`. `None` when no
/// installed plug-in lists a command there.
pub fn canvas_submenu(doc: &vectorcraft_doc::Document, ids: &[u64]) -> Option<Item> {
    let groups: Vec<Item> = context_commands(ContextMenu::Canvas, doc, ids)
        .into_iter()
        .map(|(name, commands)| Item::Sub(name, commands.into_iter().map(|c| Item::Cmd(c.label, c.command, json!({ "ids": ids }))).collect()))
        .collect();
    (!groups.is_empty()).then_some(Item::Sub("Plug-ins", groups))
}

/// Can the canvas context menu's item for plug-in command `id` with `p` be chosen? Only an item
/// that carries `ids` ([`canvas_submenu`]) has a condition; every other item can.
pub fn canvas_item_enabled(app: &VectorcraftApp, id: &str, p: &Value) -> bool {
    let Some(ids) = p.get("ids").and_then(Value::as_array) else { return true };
    let Some((_, c)) = vectorcraft_engine::cmd::plugin::command_of(id) else { return true };
    let Some(place) = c.context.iter().find(|p| p.menu == ContextMenu::Canvas) else { return true };
    let ids: Vec<u64> = ids.iter().filter_map(Value::as_u64).collect();
    app.session.active().is_some_and(|st| context_holds(&st.doc, &ids, place.when))
}

/// Object › Plug-ins: the installed object filters (each with its commands, if it has any), then
/// Install Plug-in… and Reload Plug-ins.
pub fn object_menu() -> Vec<Item> {
    let mut items: Vec<Item> = registry::list()
        .iter()
        .filter(|p| p.manifest().kind == Kind::Filter)
        .map(|p| with_commands(p, Item::Cmd(label(&p.manifest().name, !p.manifest().params.is_empty()), "plugin.dialog", json!({ "id": p.id() }))))
        .collect();
    if items.is_empty() {
        items.push(Item::Header(NO_FILTERS));
    }
    items.extend([
        Item::Sep,
        Item::Cmd("Install Plug-in…", "ui.installPlugin", Value::Null),
        Item::Cmd("Reload Plug-ins", "plugin.reload", Value::Null),
    ]);
    items
}

/// File › Open With's header while no importer plug-in is installed.
pub const NO_IMPORTERS: &str = "No importer plug-ins installed";

/// File › Open With: the installed importers (`file.openWith {importer}`), each by its name.
pub fn open_with_menu() -> Vec<Item> {
    let items: Vec<Item> = vectorcraft_plugins::import::installed()
        .iter()
        .map(|p| Item::Cmd(label(&p.manifest().name, true), "file.openWith", json!({ "importer": vectorcraft_plugins::export::format_id(p.id()) })))
        .collect();
    if items.is_empty() { vec![Item::Header(NO_IMPORTERS)] } else { items }
}

/// Effect › Plug-ins: the live-effect plug-ins, each with its commands if it has any (`None`
/// while none is installed). Effects without options apply at once, as built-in ones do.
pub fn effect_menu() -> Option<Item> {
    let items: Vec<Item> = vectorcraft_effects::plugin_effects()
        .into_iter()
        .map(|e| {
            let own = match e.defaults.as_object().is_some_and(|o| o.is_empty()) {
                true => Item::Cmd(e.label, "effect.apply", json!({ "effect": e.id })),
                false => Item::Cmd(e.label, "effect.dialog", json!({ "effect": e.id })),
            };
            match vectorcraft_plugins::effect::plugin_id(e.id).and_then(registry::get) {
                Some(plugin) => with_commands(&plugin, own),
                None => own,
            }
        })
        .collect();
    (!items.is_empty()).then_some(Item::Sub("Plug-ins", items))
}

/// `plugin.dialog {id, params?}`: run filter `id` at once when it takes no parameters or `params`
/// are given, else open its dialog prefilled with its defaults.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_str).ok_or("missing `id` (see plugin.list)")?;
    let plugin = registry::get(id).ok_or_else(|| format!("no plug-in {id:?} is installed"))?;
    let m = plugin.manifest();
    match m.kind {
        Kind::Filter => {}
        Kind::Effect => return Err(format!("{} is a live effect (Effect › Plug-ins)", m.name)),
        Kind::Exporter => return Err(format!("{} is an exporter (File › Export › Export As…)", m.name)),
        Kind::Importer => return Err(format!("{} is an importer (File › Open With)", m.name)),
    }
    let given = p.get("params").filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
    if m.params.is_empty() || given.is_some() {
        return run(app, &plugin, json!({"id": id, "params": given.cloned().unwrap_or(Value::Null)}));
    }
    let mut fields = fields_for(m);
    if m.budget == Budget::Light {
        fields.insert("preview".into(), json!(true));
    }
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(Value::Null)
}

/// The fields of plug-in `m`'s dialog: its parameters at their defaults, `__plugin` (its id) and
/// `__label` (its name, the heading).
fn fields_for(m: &Manifest) -> Map<String, Value> {
    let mut fields = m.defaults();
    fields.insert("__plugin".into(), json!(m.id));
    fields.insert("__label".into(), json!(m.name.trim_end_matches('…')));
    fields
}

/// The plug-in dialog `d` is for, its description drawn above the fields. `None` once it was
/// uninstalled, which closes the dialog.
fn described(ui: &mut egui::Ui, d: &Dialog) -> Option<Arc<Plugin>> {
    let plugin = registry::get(&d.str("__plugin"))?;
    let m = plugin.manifest();
    if !m.description.is_empty() {
        ui.label(egui::RichText::new(&m.description).color(crate::theme::Tokens::get(ui.ctx()).text_dim));
        ui.add_space(8.0);
    }
    Some(plugin)
}

/// Opens the dialog of plug-in command `id` (`plugin.<plug-in id>.<command id>`), given no params
/// or only `ids` (a context menu's objects), when it has parameters and a document is open: its
/// fields at their defaults (`__command` is the command id, `__ids` the objects given) with a live
/// Preview, and OK keeps the run on those objects as one undo step. False for any other command or
/// params, which then run as they were given.
pub fn open_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> bool {
    let only_ids = p.as_object().is_none_or(|o| o.keys().all(|k| k == "ids")) && p.get("ids").is_none_or(Value::is_array);
    if !only_ids {
        return false;
    }
    let Some((plugin, c)) = vectorcraft_engine::cmd::plugin::command_of(id) else { return false };
    if c.params.is_empty() || app.session.active().is_none() {
        return false;
    }
    let mut fields = fields_for(&plugin.manifest().for_command(&c));
    fields.insert("__command".into(), json!(id));
    if let Some(ids) = p.get("ids") {
        fields.insert("__ids".into(), ids.clone());
    }
    fields.insert("__label".into(), json!(c.shown_label()));
    fields.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    true
}

/// The plug-in command dialog `d` is for (`__command`), and the manifest it runs with; `None` for
/// a filter's own dialog.
fn command_of(d: &Dialog) -> Option<Manifest> {
    let (plugin, c) = vectorcraft_engine::cmd::plugin::command_of(&d.str("__command"))?;
    Some(plugin.manifest().for_command(&c))
}

/// The command dialog `d` runs: its plug-in command, or `plugin.run`.
fn command(d: &Dialog) -> String {
    match d.str("__command") {
        c if c.is_empty() => CMD.to_string(),
        c => c,
    }
}

/// The params of the command dialog `d` runs ([`command`]): a command opened from a context menu
/// runs on the objects it was opened for (`__ids`).
fn params(d: &Dialog) -> Value {
    match d.str("__command").is_empty() {
        true => json!({"id": d.str("__plugin"), "params": form::params(d)}),
        false => match d.fields.get("__ids") {
            Some(ids) => json!({"ids": ids, "params": form::params(d)}),
            None => json!({"params": form::params(d)}),
        },
    }
}

/// Does filter `m` run in the background ([`crate::plugin_runs`]) rather than on the UI thread? A
/// heavy one does, and so does a light one that reports its progress (the `progress` permission)
/// unless it calls commands, which run only in the session.
fn in_background(m: &Manifest) -> bool {
    m.budget == Budget::Heavy || (m.permissions().contains(&permissions::Permission::Progress) && m.command_allowlist.is_empty())
}

/// Runs filter `plugin` with `plugin.run`'s params `p`: at once, or in the background
/// ([`in_background`]).
fn run(app: &mut VectorcraftApp, plugin: &Plugin, p: Value) -> Result<Value, String> {
    match in_background(plugin.manifest()) {
        false => app.run(CMD, p),
        true => crate::plugin_runs::start(app, p),
    }
}

/// The plug-in's description and fields (a command's: its own parameters), then the live
/// Preview; a heavy filter has a note instead that it runs in the background, since a run may take
/// minutes. A command dialog closes once the plug-in no longer has the command.
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let Some(plugin) = described(ui, d) else { return true };
    let run_with = command_of(d);
    if run_with.is_none() && !d.str("__command").is_empty() {
        return true;
    }
    form::schema_fields(app, ui, d, run_with.as_ref().unwrap_or(plugin.manifest()));
    match plugin.manifest().budget {
        Budget::Light => {
            let (label, p) = (d.str("__label"), params(d));
            form::preview(app, ui, d, &label, &command(d), p);
        }
        // No Preview: each run may take minutes.
        Budget::Heavy => {
            ui.add_space(8.0);
            let note = tl!("It may take minutes, so it runs in the background, where you can cancel it.");
            ui.label(egui::RichText::new(note).color(crate::theme::Tokens::get(ui.ctx()).text_dim));
        }
    }
    false
}

/// OK: a filter keeps its previewed run as one undo step; a heavy one, which has no preview, and a
/// light one that runs in the background ([`in_background`]) with Preview off or a failed preview,
/// start in the background ([`run`]) and the dialog closes. A preview still shown when Preview was
/// just turned off, or a preview that failed, is rolled back first.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let own_run = d.str("__command").is_empty();
    let kept = |plugin: &Plugin| {
        plugin.manifest().budget == Budget::Light && d.bool("preview") && app.session.in_interaction() && !d.fields.contains_key(form::REFUSED)
    };
    match registry::get(&d.str("__plugin")) {
        Some(plugin) if own_run && in_background(plugin.manifest()) && !kept(&plugin) => {
            if d.fields.contains_key(form::PREVIEWED) && app.session.in_interaction() {
                let _ = app.session.cancel_interaction();
            }
            let r = run(app, &plugin, params(d));
            if r.is_ok() {
                app.ui.dialog = None;
            }
            r
        }
        _ => form::commit_preview(app, &command(d), params(d)),
    }
}

/// The dialog kind of an exporter's parameters, after Export As picked the file.
pub const EXPORT_KIND: &str = "pluginExport";

/// Headed by the plug-in's name, as it is; Export writes the file.
pub(super) const EXPORT_SPEC: DialogSpec = DialogSpec {
    heading: |d| d.str("__label"),
    body: export_body,
    confirm: export_confirm,
    ok: Some("Export"),
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

/// Export As with exporter `plugin`, which has parameters: its fields at their defaults, and
/// `export`, the export's own parameters (`format`, `path`, …), kept for Export.
pub fn open_export(app: &mut VectorcraftApp, plugin: &Plugin, export: Value) {
    let mut fields = fields_for(plugin.manifest());
    fields.insert("__export".into(), export);
    app.ui.dialog = Some(Dialog { kind: EXPORT_KIND.into(), fields });
}

fn export_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let Some(plugin) = described(ui, d) else { return true };
    form::schema_fields(app, ui, d, plugin.manifest());
    false
}

/// Writes the file with the export's parameters and the plug-in's, as `params`.
fn export_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = d.fields.get("__export").cloned().filter(Value::is_object).ok_or("no format to export to")?;
    p["params"] = form::params(d);
    let format = p.get("format").and_then(Value::as_str).map(str::to_string);
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    app.ui.dialog = None;
    crate::io::export(app, format.as_deref(), path, &p)
}

/// The dialog kind of an importer's parameters, after Open With picked the file.
pub const IMPORT_KIND: &str = "pluginImport";

/// Headed by the plug-in's name, as it is; Open opens the file.
pub(super) const IMPORT_SPEC: DialogSpec = DialogSpec {
    heading: |d| d.str("__label"),
    body: export_body,
    confirm: import_confirm,
    ok: Some("Open"),
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

/// Open With with importer `plugin`, which has parameters: its fields at their defaults, and
/// `open`, Open With's own parameters (`path`, `importer`, and `picked` for a file from the
/// picker, which is read off the UI thread), kept for Open.
pub fn open_import(app: &mut VectorcraftApp, plugin: &Plugin, open: Value) {
    let mut fields = fields_for(plugin.manifest());
    fields.insert("__open".into(), open);
    app.ui.dialog = Some(Dialog { kind: IMPORT_KIND.into(), fields });
}

/// Opens the file with Open With's parameters and the plug-in's, as `params`.
fn import_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = d.fields.get("__open").cloned().filter(Value::is_object).ok_or("no file to open")?;
    p["params"] = form::params(d);
    app.ui.dialog = None;
    let picked = p["picked"] == true;
    crate::io::open_with_on(app, &p, picked && crate::opening::available())
}

/// The dialog kind of the approval of a plug-in install.
pub const INSTALL_KIND: &str = "pluginInstall";

/// Who asked for a plug-in install, fixed when the request is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallOrigin {
    /// The person at the keyboard: Install Plug-in…, File › Open or a drop.
    Person,
    /// A control or MCP request, a frame of synthetic input, or a file dialog that one of them
    /// opened ([`VectorcraftApp::automation_asked`]): nothing in the dialog installs it, and the
    /// person starts Install Plug-in… to install it.
    Automation,
}

/// What [`ask_install`] answers, as `approval`, for a question automation asked: the person has to
/// start Install Plug-in… to install the module.
pub(crate) const PERSON_MUST_START_INSTALL: &str = "personMustStartInstall";

/// A plug-in install waiting for a person's answer.
pub(crate) struct PendingInstall {
    /// The question and the lines under it, in the UI language.
    pub message: String,
    pub detail: String,
    /// `plugin.install`'s params for OK.
    pub params: Value,
    /// What OK approves: the module the question is about.
    pub approval: InstallApproval,
    /// Who asked; a question automation asked has no OK that installs.
    pub origin: InstallOrigin,
    /// How long the question has been on screen; [`ask_install`] starts it over.
    pub shown: Shown,
}

/// How long, in seconds, the approval's question stays on screen in the focused window, on every
/// frame and unchanged, before its OK takes a click, Enter or Space.
pub(crate) const APPROVAL_DELAY: f64 = 0.5;

/// How long the approval's question has been on screen, from the frames that drew it.
#[derive(Default)]
pub(crate) struct Shown {
    /// The time of the first frame of the current showing: drawn, visible, in the focused window,
    /// right after a frame that drew it too. `None` while it isn't shown.
    since: Option<f64>,
    /// The pass number and time of the last frame that drew it.
    pass: Option<u64>,
    last: Option<f64>,
    /// A pointer button, Enter or Space was down on a frame where OK was disabled, and they haven't
    /// all been let go of since.
    pressed_early: bool,
    /// OK takes input: shown for [`APPROVAL_DELAY`], with no press from before that held.
    ready: bool,
}

impl Shown {
    /// Notes a frame that draws the question in `ui` → whether OK is enabled on it. The showing
    /// starts over when the frame before didn't draw it, when time went back, when the window lost
    /// the focus since the frame before, and on any frame that is a measuring pass, is invisible or
    /// comes while the window doesn't have the focus.
    fn drawn(&mut self, ui: &egui::Ui) -> bool {
        let pass = ui.ctx().cumulative_pass_nr();
        let (now, focused, left, pressed) = ui.input(|i| {
            let left = i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)));
            let pressed = i.pointer.any_pressed() || [egui::Key::Enter, egui::Key::Space].into_iter().any(|k| i.key_pressed(k));
            (i.time, i.focused, left, pressed || Self::held(i))
        });
        let seen = focused && ui.is_visible() && !ui.is_sizing_pass() && now.is_finite();
        let follows = self.pass.is_some_and(|p| p.checked_add(1) == Some(pass)) && self.last.is_some_and(|t| t <= now);
        if !seen || !follows || left {
            self.since = None;
        }
        if seen && self.since.is_none() {
            self.since = Some(now);
        }
        (self.pass, self.last) = (Some(pass), Some(now));
        let enabled = self.since.is_some_and(|s| now - s >= APPROVAL_DELAY);
        if pressed && !enabled {
            self.pressed_early = true;
        }
        self.ready = enabled && !self.pressed_early;
        if let Some(s) = self.since.filter(|_| !enabled) {
            ui.ctx().request_repaint_after_secs((s + APPROVAL_DELAY - now) as f32);
        }
        enabled
    }

    /// After the frame's OK: a press from before OK took input stops counting once every pointer
    /// button, Enter and Space are up.
    fn released(&mut self, ui: &egui::Ui) {
        if !ui.input(Self::held) {
            self.pressed_early = false;
        }
    }

    /// Whether a pointer button, Enter or Space is down: what can click OK when it is let go of or
    /// repeats.
    fn held(i: &egui::InputState) -> bool {
        i.pointer.any_down() || i.key_down(egui::Key::Enter) || i.key_down(egui::Key::Space)
    }

    /// Whether OK takes input: the question was shown for [`APPROVAL_DELAY`] on the last frame
    /// that drew it.
    pub(crate) fn ready(&self) -> bool {
        self.ready
    }
}

/// No OK in the shared footer: the OK in its body is a `Sense::click` button, which Enter clicks
/// only when it has the keyboard focus.
pub(super) const INSTALL_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Install Plug-in…").trim_end_matches('…').to_string(),
    body_with_actions: Some(install_body),
    confirm: install_confirm,
    ok: None,
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

/// Asks a person whether to install what `pending` describes, its question not yet shown, even when
/// it reads as the one it replaces. → `{pending}`, and `approval: "personMustStartInstall"` when
/// automation asked.
pub(crate) fn ask_install(app: &mut VectorcraftApp, mut pending: PendingInstall) -> Value {
    pending.shown = Shown::default();
    let automated = pending.origin == InstallOrigin::Automation;
    app.pending_plugin_install = Some(pending);
    app.ui.dialog = Some(Dialog::new(INSTALL_KIND, json!({})));
    if automated {
        return json!({ "pending": INSTALL_KIND, "approval": PERSON_MUST_START_INSTALL });
    }
    json!({ "pending": INSTALL_KIND })
}

/// What the approval says when automation asked for the install; the error of its OK
/// ([`install_confirm`]) is the same text.
fn automation_asked_note() -> &'static str {
    tl!("Automation asked to install this plug-in. To install it, choose Object › Plug-ins › Install Plug-in… yourself.")
}

/// The question, the lines under it, and OK, which is disabled until the question has been shown
/// for [`APPROVAL_DELAY`] ([`Shown`]), and Cancel, which never waits. When automation asked, a note
/// says so and OK stays disabled.
fn install_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, _: &mut Dialog) -> DialogAction {
    let Some(pending) = app.pending_plugin_install.as_mut() else { return DialogAction::Cancel };
    let person = pending.origin == InstallOrigin::Person;
    let enabled = pending.shown.drawn(ui) && person;
    ui.label(egui::RichText::new(&pending.message).strong());
    ui.add_space(8.0);
    ui.label(egui::RichText::new(&pending.detail).color(crate::theme::Tokens::get(ui.ctx()).text_dim));
    if !person {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(automation_asked_note()).strong());
    }
    ui.add_space(16.0);
    let mut action = DialogAction::None;
    let row = egui::vec2(ui.min_rect().width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::primary_button_enabled(ui, "OK", enabled).clicked() && person && pending.shown.ready() {
            action = DialogAction::Confirm;
        }
        pending.shown.released(ui);
        ui.add_space(8.0);
        if widgets::secondary_button(ui, "Cancel").clicked() {
            action = DialogAction::Cancel;
        }
    });
    action
}

/// OK: installs the module the dialog showed, if a person pressed it; automation is refused and the
/// dialog stays open, and so is any OK of a question automation asked. Before the question has been
/// shown for [`APPROVAL_DELAY`] it installs nothing and the dialog stays open. → `{pending}`.
fn install_confirm(app: &mut VectorcraftApp, _: &Dialog) -> Result<Value, String> {
    if app.pending_plugin_install.as_ref().is_some_and(|p| p.origin == InstallOrigin::Automation) {
        return Err("Automation asked to install this plug-in. To install it, choose Object › Plug-ins › Install Plug-in… yourself.".into());
    }
    if app.automation_is_acting() {
        return Err("Only the person at the keyboard can approve installing a plug-in".into());
    }
    if app.pending_plugin_install.as_ref().is_some_and(|p| !p.shown.ready()) {
        return Ok(json!({ "pending": INSTALL_KIND }));
    }
    app.ui.dialog = None;
    let Some(pending) = app.pending_plugin_install.take() else { return Ok(Value::Null) };
    app.session.approve_plugin_install(pending.approval);
    let r = app.run("plugin.install", pending.params);
    app.session.clear_plugin_install_approval();
    let r = r?;
    crate::io::report_installed(app, &r);
    Ok(r)
}
