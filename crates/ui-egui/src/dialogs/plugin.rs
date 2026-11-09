//! WebAssembly plug-ins in the menus and their generated dialogs.
//!
//! Object › Plug-ins lists the installed object filters (`plugin.dialog {id}`), then Install
//! Plug-in… and Reload Plug-ins; Effect › Plug-ins lists the live-effect plug-ins, which open the
//! effect dialog (`effect.dialog`). A filter with parameters opens dialog `plugin`: one field per
//! declared parameter (fields named after them; `__plugin` is the plug-in id) with a live
//! Preview, and OK keeps the run as one undo step (`plugin.run`). A filter without parameters runs
//! at once. A heavy filter (its manifest's `budget`) has no Preview, since a run may take minutes:
//! it runs in the background ([`crate::plugin_runs`]). A filter or live effect with commands of
//! its own is a submenu named after it, holding its item and then its commands; a command with
//! parameters opens the same dialog for them ([`open_command`]). Exporters are formats of File › Export ›
//! Export As; one with parameters opens dialog `pluginExport` once the file is picked, and its
//! Export button writes the file. Importers open the files File › Open finds no built-in format
//! for, and File › Open With lists them, to open a file with one of them by name; one with
//! parameters opens dialog `pluginImport` once the file is picked, and its Open button opens it.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_plugins::{Budget, Kind, Manifest, Plugin, permissions, registry};

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::menus::Item;
use crate::state::Dialog;

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

/// Opens the dialog of plug-in command `id` (`plugin.<plug-in id>.<command id>`), given no params,
/// when it has parameters and a document is open: its fields at their defaults (`__command` is the
/// command id) with a live Preview, and OK keeps the run as one undo step. False for any other
/// command, which then runs as it was given.
pub fn open_command(app: &mut VectorcraftApp, id: &str) -> bool {
    let Some((plugin, c)) = vectorcraft_engine::cmd::plugin::command_of(id) else { return false };
    if c.params.is_empty() || app.session.active().is_none() {
        return false;
    }
    let mut fields = fields_for(&plugin.manifest().for_command(&c));
    fields.insert("__command".into(), json!(id));
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

/// The params of the command dialog `d` runs ([`command`]).
fn params(d: &Dialog) -> Value {
    match d.str("__command").is_empty() {
        true => json!({"id": d.str("__plugin"), "params": form::params(d)}),
        false => json!({"params": form::params(d)}),
    }
}

/// Runs filter `plugin` with `plugin.run`'s params `p`: at once, or in the background when it is
/// heavy.
fn run(app: &mut VectorcraftApp, plugin: &Plugin, p: Value) -> Result<Value, String> {
    match plugin.manifest().budget {
        Budget::Light => app.run(CMD, p),
        Budget::Heavy => crate::plugin_runs::start(app, p),
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

/// OK: a filter keeps its previewed run as one undo step; a heavy one, which has no preview, starts
/// in the background ([`run`]) and the dialog closes.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    match registry::get(&d.str("__plugin")) {
        Some(plugin) if plugin.manifest().budget == Budget::Heavy => {
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
