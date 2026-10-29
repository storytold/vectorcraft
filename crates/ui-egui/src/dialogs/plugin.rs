//! WebAssembly plug-ins in the menus and their generated dialogs.
//!
//! Object › Plug-ins lists the installed object filters (`plugin.dialog {id}`), then Install
//! Plug-in… and Reload Plug-ins; Effect › Plug-ins lists the live-effect plug-ins, which open the
//! effect dialog (`effect.dialog`). A filter with parameters opens dialog `plugin`: one field per
//! declared parameter (fields named after them; `__plugin` is the plug-in id) with a live
//! Preview, and OK keeps the run as one undo step (`plugin.run`). A filter without parameters runs
//! at once. A heavy filter (its manifest's `budget`) has no Preview, since a run may take minutes:
//! it runs in the background ([`crate::plugin_runs`]). Exporters are formats of File › Export ›
//! Export As; one with parameters opens dialog `pluginExport` once the file is picked, and its
//! Export button writes the file.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_plugins::{Budget, Kind, Manifest, Plugin, registry};

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

/// Object › Plug-ins: the installed object filters, then Install Plug-in… and Reload Plug-ins.
pub fn object_menu() -> Vec<Item> {
    let mut items: Vec<Item> = registry::list()
        .iter()
        .filter(|p| p.manifest().kind == Kind::Filter)
        .map(|p| Item::Cmd(label(&p.manifest().name, !p.manifest().params.is_empty()), "plugin.dialog", json!({ "id": p.id() })))
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

/// Effect › Plug-ins: the live-effect plug-ins (`None` while none is installed). Effects without
/// options apply at once, as built-in ones do.
pub fn effect_menu() -> Option<Item> {
    let items: Vec<Item> = vectorcraft_effects::plugin_effects()
        .into_iter()
        .map(|e| match e.defaults.as_object().is_some_and(|o| o.is_empty()) {
            true => Item::Cmd(e.label, "effect.apply", json!({ "effect": e.id })),
            false => Item::Cmd(e.label, "effect.dialog", json!({ "effect": e.id })),
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

fn params(d: &Dialog) -> Value {
    json!({"id": d.str("__plugin"), "params": form::params(d)})
}

/// Runs filter `plugin` with `plugin.run`'s params `p`: at once, or in the background when it is
/// heavy.
fn run(app: &mut VectorcraftApp, plugin: &Plugin, p: Value) -> Result<Value, String> {
    match plugin.manifest().budget {
        Budget::Light => app.run(CMD, p),
        Budget::Heavy => crate::plugin_runs::start(app, p),
    }
}

/// The plug-in's description and fields, then the live Preview; a heavy filter has a note instead
/// that it runs in the background, since a run may take minutes.
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let Some(plugin) = described(ui, d) else { return true };
    form::schema_fields(app, ui, d, plugin.manifest());
    match plugin.manifest().budget {
        Budget::Light => {
            let (label, p) = (d.str("__label"), params(d));
            form::preview(app, ui, d, &label, CMD, p);
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
        _ => form::commit_preview(app, CMD, params(d)),
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
