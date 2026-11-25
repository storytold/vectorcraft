//! WebAssembly plug-ins in the menus and their generated dialogs.
//!
//! Object › Plug-ins lists the installed object filters and generators (`plugin.dialog {id}`),
//! then Edit Generated Art…, Install Plug-in… and Reload Plug-ins. A generator with parameters
//! opens a hosted dialog `pluginGenerate`, whose OK draws new generated art (`plugin.generate`);
//! Edit Generated Art… opens it with a selected generated art object's parameters, and its OK draws
//! the object again (`plugin.editGenerated`). Effect › Plug-ins lists the live-effect plug-ins,
//! which open the effect dialog (`effect.dialog`). A filter with parameters opens a hosted dialog `plugin`: one row
//! per declared parameter, shown while its `visibleWhen` holds, with a live Preview, and OK keeps the
//! run as one undo step (`plugin.run`). A filter without parameters runs at once. A heavy filter (its
//! manifest's `budget`) has no Preview, since a run may take minutes: it runs in the background
//! ([`crate::plugin_runs`]). A light filter that reports its progress (plug-in API level 2 with the
//! `progress` permission) and calls no commands runs in the background too, with its progress and
//! Cancel, when it runs at once or when OK runs it without a kept preview ([`in_background`]). A
//! filter or live effect with commands of its own is a submenu named after it, holding its item and
//! then its commands; a command with parameters opens the same dialog for them ([`open_command`]).
//! A command whose manifest names another menu is listed in that menu instead ([`place_commands`]).
//! Exporters are formats of File › Export › Export As; one with parameters opens dialog
//! `pluginExport` once the file is picked, and its Export button writes the file. Importers open the
//! files File › Open finds no built-in format for, and File › Open With lists them; one with
//! parameters opens dialog `pluginImport` once the file is picked, and its Open button opens it.
//!
//! The one open filter, generator, export or import dialog is [`PluginDialog`]: what the hosted
//! dialog cannot hold (the plug-in, the live preview, the edited object, the export's or open's own
//! parameters) lives there, and
//! [`pump`] reads the dialog's values each frame and acts on its answer.
//!
//! [`InstallPrompt`] asks a person to approve a plug-in's permissions or heavy budget. What it
//! shows and installs is held by the app, not by the prompt; it takes no programmatic answer, so a
//! control request or synthetic input can't confirm it. Its OK takes no click, Enter or Space until
//! the question has been on screen in the focused window for [`APPROVAL_DELAY`], and that time
//! starts over for every new question and whenever the window loses the focus.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use craft::menu::Node;
use craft::ui::contract::Show as _;
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, Prompt, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog};
use craft::ui::runtime::windows::WindowHost;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::plugin::InstallApproval;
use vectorcraft_plugins::{Budget, Kind, Manifest, Plugin, permissions, registry};

use super::form;
use super::preview::LivePreview;
use crate::VectorcraftApp;
use crate::menus::{Sep, c, cp, placeholder, sub};

/// The key of the hosted dialog of plug-in filters.
pub const KIND: &str = "plugin";

const CMD: &str = "plugin.run";

/// Object › Plug-ins' disabled row while no filter plug-in is installed.
pub const NO_FILTERS: &str = "No filter plug-ins installed";

/// A menu label for plug-in `name`: with an ellipsis when it opens a dialog.
fn label(name: &str, has_params: bool) -> &'static str {
    let dots = if has_params && !name.ends_with('…') { "…" } else { "" };
    registry::intern(&format!("{name}{dots}"))
}

/// A filter or live effect `plugin`'s item `own`, as the menu lists it: alone, or for a plug-in
/// with commands of its own, a submenu named after it holding `own` and then its commands. Each
/// command runs its command id (`plugin.<id>.<command>`), with an ellipsis when it has parameters,
/// whose dialog [`open_command`] opens.
fn with_commands(plugin: &Plugin, own: Node) -> Node {
    let m = plugin.manifest();
    if m.commands.is_empty() {
        return own;
    }
    let mut items = vec![own, Sep];
    items.extend(m.commands.iter().map(|command| {
        let id = registry::intern(&vectorcraft_engine::cmd::plugin::command_id(&m.id, &command.id));
        c(label(&command.shown_label(), !command.params.is_empty()), id)
    }));
    sub(registry::intern(&permissions::display_text(m.name.trim(), permissions::MAX_SHOWN_NAME)), items)
}

/// Moves each plug-in command whose manifest's `menu` names another menu
/// ([`vectorcraft_plugins::MenuPlace`]) from its plug-in's submenu into that menu of `tree`: after
/// the built-in item its `after` names (with or without an ellipsis), or at the end of the menu.
/// Its label ends with the plug-in's name ([`vectorcraft_plugins::Command::menu_label`]).
/// Commands placed at the same spot are listed in plug-in id order and then in manifest order. A
/// plug-in's submenu left with its own item alone becomes that item. A command whose menu `tree`
/// doesn't have, or that [`vectorcraft_engine::cmd::plugin::placed_menu`] leaves in its plug-in's
/// submenu, stays there.
pub fn place_commands(tree: &mut [(&'static str, Vec<Node>)]) {
    let mut spots: Vec<(&'static [&'static str], Option<String>, Vec<Node>)> = Vec::new();
    for p in registry::list() {
        let m = p.manifest();
        if !matches!(m.kind, Kind::Filter | Kind::Effect) {
            continue;
        }
        for command in &m.commands {
            let Some(place) = &command.place else { continue };
            if vectorcraft_engine::cmd::plugin::placed_menu(&p, command).is_none() {
                continue;
            }
            // An `after` that names no built-in item of the menu means the end of it, as no `after` does.
            // The spot is the item's own label, so both labels of an item that switches are one spot.
            let after = place.after.as_deref().and_then(|a| {
                let menu = menu_at(tree, place.path)?;
                menu.iter().find(|it| is_anchor(it, anchor_key(a))).and_then(own_key).map(str::to_string)
            });
            let id = registry::intern(&vectorcraft_engine::cmd::plugin::command_id(&m.id, &command.id));
            let item = c(label(&command.menu_label(&m.name), !command.params.is_empty()), id);
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
    let placed: std::collections::HashSet<String> = spots
        .iter()
        .flat_map(|s| s.2.iter())
        .filter_map(|it| match it {
            Node::Item(i) => Some(i.command.id.clone()),
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
fn is_anchor(it: &Node, key: &str) -> bool {
    let other = |l: &str| crate::menus::DYNAMIC_LABELS.contains(&l) && vectorcraft_engine::cmd::plugin::other_state(l).is_some_and(|o| o == key);
    own_key(it).is_some_and(|own| own == key || matches!(it, Node::Item(i) if other(&i.label)))
}

/// The label of `it` as [`anchor_key`] reads it, when `it` is a built-in item or submenu that can be
/// an anchor; `None` for an item that lists a plug-in.
fn own_key(it: &Node) -> Option<&str> {
    match it {
        Node::Item(i) if !crate::menus::lists_plugin(&i.command.id, &i.command.params) => Some(anchor_key(&i.label)),
        Node::Submenu { label, children, .. } if !crate::menus::holds_plugin_commands(children) => Some(anchor_key(label)),
        _ => None,
    }
}

/// The built-in menu of `tree` at `path`: a top-level menu, then its submenus by label.
fn menu_at<'a>(tree: &'a mut [(&'static str, Vec<Node>)], path: &[&str]) -> Option<&'a mut Vec<Node>> {
    let (first, rest) = path.split_first()?;
    let mut menu = &mut tree.iter_mut().find(|(title, _)| title == first)?.1;
    for segment in rest {
        menu = menu.iter_mut().find_map(|it| match it {
            Node::Submenu { label, children, .. } if label.as_str() == *segment && !crate::menus::holds_plugin_commands(children) => Some(children),
            _ => None,
        })?;
    }
    Some(menu)
}

/// Takes the commands in `placed` out of the plug-in submenus under `items`; a submenu left with no
/// command becomes its plug-in's own item.
fn leave_out(items: &mut [Node], placed: &std::collections::HashSet<String>) {
    for it in items.iter_mut() {
        let Node::Submenu { children, .. } = it else { continue };
        if !crate::menus::holds_plugin_commands(children) {
            leave_out(children, placed);
            continue;
        }
        children.retain(|c| !matches!(c, Node::Item(i) if placed.contains(&i.command.id)));
        if !children.iter().any(|c| matches!(c, Node::Item(i) if vectorcraft_engine::cmd::plugin::is_command_id(&i.command.id)))
            && let Some(own) = children.first().cloned()
        {
            *it = own;
        }
    }
}

/// Object › Plug-ins: the installed object filters (each with its commands, if it has any) and
/// generators, then Edit Generated Art…, Install Plug-in… and Reload Plug-ins.
pub fn object_menu() -> Vec<Node> {
    let mut items: Vec<Node> = registry::list()
        .iter()
        .filter(|p| matches!(p.manifest().kind, Kind::Filter | Kind::Generator))
        .map(|p| with_commands(p, cp(label(&p.manifest().name, !p.manifest().params.is_empty()), "plugin.dialog", json!({ "id": p.id() }))))
        .collect();
    if items.is_empty() {
        items.push(placeholder(NO_FILTERS));
    }
    items.extend([Sep, c(EDIT_GENERATED, EDIT_COMMAND), c("Install Plug-in…", "ui.installPlugin"), c("Reload Plug-ins", "plugin.reload")]);
    items
}

/// File › Open With's disabled row while no importer plug-in is installed.
pub const NO_IMPORTERS: &str = "No importer plug-ins installed";

/// File › Open With: the installed importers (`file.openWith {importer}`), each by its name.
pub fn open_with_menu() -> Vec<Node> {
    let items: Vec<Node> = vectorcraft_plugins::import::installed()
        .iter()
        .map(|p| cp(label(&p.manifest().name, true), "file.openWith", json!({ "importer": vectorcraft_plugins::export::format_id(p.id()) })))
        .collect();
    if items.is_empty() { vec![placeholder(NO_IMPORTERS)] } else { items }
}

/// Effect › Plug-ins: the live-effect plug-ins, each with its commands if it has any (`None`
/// while none is installed). Effects without options apply at once, as built-in ones do.
pub fn effect_menu() -> Option<Node> {
    let items: Vec<Node> = vectorcraft_effects::plugin_effects()
        .into_iter()
        .map(|e| {
            let own = match e.defaults.as_object().is_some_and(|o| o.is_empty()) {
                true => cp(e.label, "effect.apply", json!({ "effect": e.id })),
                false => cp(e.label, "effect.dialog", json!({ "effect": e.id })),
            };
            match vectorcraft_plugins::effect::plugin_id(e.id).and_then(registry::get) {
                Some(plugin) => with_commands(&plugin, own),
                None => own,
            }
        })
        .collect();
    (!items.is_empty()).then_some(sub("Plug-ins", items))
}

/// The open filter, export or import dialog: its id on the host and what the app keeps beside it.
pub(crate) struct PluginDialog {
    id: DialogId,
    kind: Opened,
}

/// What a [`PluginDialog`] is for. `params` is every row the plug-in declares, hidden ones too: the
/// dialog shows only those whose `visibleWhen` holds, and its values keep the rest.
pub(crate) enum Opened {
    /// A filter's own dialog (`command` is `None`), or the dialog of one of its commands.
    Filter {
        plugin: String,
        command: Option<String>,
        /// The undo step's name, as the preview and the commit report it.
        label: String,
        preview: LivePreview,
        /// Whether the dialog has a Preview.
        light: bool,
        /// Whether the Preview check box was on at the last pass.
        previewing: bool,
        /// Whether the last preview failed.
        refused: bool,
        params: Arc<[Param]>,
    },
    /// A generator's parameters: for new generated art (`edit` is `None`), or for drawing the
    /// generated art object `edit` names again.
    Generate { plugin: String, edit: Option<EditTarget>, params: Arc<[Param]> },
    /// An exporter's parameters, with Export As's own parameters (`format`, `path`, …) kept for Export.
    Export { plugin: String, export: Value, params: Arc<[Param]> },
    /// An importer's parameters, with Open With's own parameters (`path`, `importer`, `picked`) kept for Open.
    Import { plugin: String, open: Value, params: Arc<[Param]> },
}

impl PluginDialog {
    /// Whether the dialog is an importer's whose Open opens a file the picker chose, which is read off the UI thread
    /// where the app can.
    #[cfg(test)]
    pub(crate) fn opens_picked(&self) -> bool {
        matches!(&self.kind, Opened::Import { open, .. } if open["picked"] == true)
    }
}

impl Opened {
    fn plugin(&self) -> &str {
        match self {
            Opened::Filter { plugin, .. } | Opened::Generate { plugin, .. } | Opened::Export { plugin, .. } | Opened::Import { plugin, .. } => plugin,
        }
    }

    fn params(&self) -> &Arc<[Param]> {
        match self {
            Opened::Filter { params, .. } | Opened::Generate { params, .. } | Opened::Export { params, .. } | Opened::Import { params, .. } => params,
        }
    }

    /// Whether the plug-in, or the command the dialog is for, is no longer installed.
    fn gone(&self) -> bool {
        match self {
            Opened::Filter { command: Some(c), .. } => vectorcraft_engine::cmd::plugin::command_of(c).is_none(),
            other => registry::get(other.plugin()).is_none(),
        }
    }
}

/// How a hosted plug-in dialog looks.
struct Look {
    /// The heading: the plug-in's or command's label, as it is (plug-in text is never translated).
    title: String,
    /// The plug-in's description, drawn above the rows.
    description: String,
    /// The Preview check box.
    previews: bool,
    /// The note of a heavy filter that it runs in the background.
    note: bool,
    /// The accepting button's label when it is not OK.
    ok: Option<&'static str>,
}

/// The builder of a hosted plug-in dialog over its values: the rows of `all` whose parameter is
/// shown for the values (`m.visible`), after the description.
fn build(key: &'static str, m: Manifest, all: Arc<[Param]>, look: Look) -> impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static {
    move |fields| {
        let shown = m.visible(&form::json_of(&all, fields));
        let mut rows = Vec::with_capacity(all.len() + 1);
        if !look.description.is_empty() {
            rows.push(Param::read_only_text("description", "", look.description.clone()).full_width());
        }
        rows.extend(all.iter().filter(|p| shown.contains(p.key.as_str())).cloned());
        let mut dialog = ParamDialog::new(key, look.title.clone(), Cow::Owned(rows)).previews(look.previews);
        if let Some(ok) = look.ok {
            dialog = dialog.ok_label(ok.into());
        }
        if look.note {
            let note = tl!("It may take minutes, so it runs in the background, where you can cancel it.");
            dialog = dialog.below(move |ui, _| {
                Text::new(note).shade(Shade::Muted).show(ui);
            });
        }
        dialog
    }
}

/// Opens the hosted dialog `key` for plug-in manifest `m` (an instance per `instance`), closing the
/// one open before → its id on the host and its rows.
fn open_hosted(app: &mut VectorcraftApp, key: &'static str, instance: &str, m: Manifest, look: Look) -> Result<(DialogId, Arc<[Param]>), String> {
    open_hosted_with(app, key, instance, m, look, &Map::new())
}

/// [`open_hosted`], with the rows of the parameters `values` holds set to those values rather than
/// their defaults.
fn open_hosted_with(
    app: &mut VectorcraftApp,
    key: &'static str,
    instance: &str,
    m: Manifest,
    look: Look,
    values: &Map<String, Value>,
) -> Result<(DialogId, Arc<[Param]>), String> {
    let (Some(ctx), Some(dialogs)) = (app.ui_ctx.clone(), app.dialogs()) else { return Err("no window".into()) };
    close_open(app);
    let mut defaults = m.defaults();
    for (k, v) in values {
        if let Some(slot) = defaults.get_mut(k) {
            *slot = v.clone();
        }
    }
    let all: Arc<[Param]> = form::plugin_params(&m, &defaults).into();
    let fields = form::fields_of(&all, &defaults);
    let dialog = HostedParamDialog::new(&ctx, key, fields, build(key, m, Arc::clone(&all), look)).instance(instance.to_string());
    let id = dialogs.with(|host| host.open(dialog));
    Ok((id, all))
}

/// Closes the open plug-in dialog, if any, rolling back its preview.
fn close_open(app: &mut VectorcraftApp) {
    let Some(old) = app.plugin_dialog.take() else { return };
    if let (Some(ctx), Some(dialogs)) = (app.ui_ctx.clone(), app.dialogs()) {
        dialogs.with(|host| host.cancel_by_id(&ctx, old.id));
    }
    if let Opened::Filter { preview, .. } = old.kind {
        preview.cancel(app);
    }
}

/// `plugin.dialog {id, params?}`: run filter or generator `id` at once when it takes no parameters
/// or `params` are given, else open its dialog prefilled with its defaults.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_str).ok_or("missing `id` (see plugin.list)")?;
    let plugin = registry::get(id).ok_or_else(|| format!("no plug-in {id:?} is installed"))?;
    let m = plugin.manifest();
    match m.kind {
        Kind::Filter => {}
        Kind::Generator => {
            let given = p.get("params").filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
            if m.params.is_empty() || given.is_some() {
                return app.run(GENERATE, json!({"id": id, "params": given.cloned().unwrap_or(Value::Null)}));
            }
            open_generate(app, &plugin, None, &Map::new())?;
            return Ok(Value::Null);
        }
        Kind::Effect => return Err(format!("{} is a live effect (Effect › Plug-ins)", m.name)),
        Kind::Exporter => return Err(format!("{} is an exporter (File › Export › Export As…)", m.name)),
        Kind::Importer => return Err(format!("{} is an importer (File › Open With)", m.name)),
    }
    let given = p.get("params").filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
    if m.params.is_empty() || given.is_some() {
        return run(app, &plugin, json!({"id": id, "params": given.cloned().unwrap_or(Value::Null)}));
    }
    open_filter(app, &plugin, m.clone(), None, m.name.trim_end_matches('…').to_string())?;
    Ok(Value::Null)
}

/// Opens the dialog of filter `plugin`, or of its command `command`, with `m` the manifest it runs
/// with, headed `heading`: the rows at their defaults, with a live Preview for a light filter.
fn open_filter(app: &mut VectorcraftApp, plugin: &Plugin, m: Manifest, command: Option<String>, heading: String) -> Result<(), String> {
    let own = plugin.manifest();
    let light = own.budget == Budget::Light;
    let look = Look { title: heading.clone(), description: own.description.clone(), previews: light, note: !light, ok: None };
    let instance = command.as_deref().unwrap_or(plugin.id()).to_string();
    let (id, params) = open_hosted(app, KIND, &instance, m, look)?;
    let kind = Opened::Filter {
        plugin: plugin.id().to_string(),
        command,
        label: heading,
        preview: Default::default(),
        light,
        previewing: light,
        refused: false,
        params,
    };
    app.plugin_dialog = Some(PluginDialog { id, kind });
    Ok(())
}

/// Opens the dialog of plug-in command `id` (`plugin.<plug-in id>.<command id>`), given no params,
/// when it has parameters and a document is open: its rows at their defaults with a live Preview, and
/// OK keeps the run as one undo step. False for any other command, which then runs as it was given.
pub fn open_command(app: &mut VectorcraftApp, id: &str) -> bool {
    let Some((plugin, c)) = vectorcraft_engine::cmd::plugin::command_of(id) else { return false };
    if c.params.is_empty() || app.session.active().is_none() {
        return false;
    }
    if let Err(e) = open_filter(app, &plugin, plugin.manifest().for_command(&c), Some(id.to_string()), c.shown_label().to_string()) {
        app.status(e);
    }
    true
}

/// The command a filter dialog runs and its params: `plugin.run` for the filter's own dialog, else
/// its plug-in command.
fn command(plugin: &str, command: Option<&str>, values: Map<String, Value>) -> (String, Value) {
    match command {
        None => (CMD.to_string(), json!({"id": plugin, "params": values})),
        Some(c) => (c.to_string(), json!({"params": values})),
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

/// The key of a generator's parameters dialog: for new generated art, or for drawing a generated
/// art object again.
pub const GENERATE_KIND: &str = "pluginGenerate";

const GENERATE: &str = "plugin.generate";

/// Object › Plug-ins › Edit Generated Art…, and the UI command it runs.
pub const EDIT_GENERATED: &str = "Edit Generated Art…";
pub const EDIT_COMMAND: &str = "ui.editGeneratedArt";

/// The generated art object an Edit Generated Art… dialog draws again: its id, and the
/// [`uid`](vectorcraft_engine::DocState::uid) of the document it is in.
pub(crate) struct EditTarget {
    object: Value,
    doc: Option<u64>,
}

/// OK's error when the document of an Edit Generated Art… dialog is no longer the active one.
const EDIT_DOC_GONE: &str = "the document it was prepared for is no longer active";

/// Opens the dialog of generator `plugin`: its rows at the parameters `values` holds, else at their
/// defaults, for new generated art, or for drawing the object of `edit` again.
fn open_generate(app: &mut VectorcraftApp, plugin: &Plugin, edit: Option<EditTarget>, values: &Map<String, Value>) -> Result<(), String> {
    let m = plugin.manifest();
    let look = Look { title: m.name.trim_end_matches('…').to_string(), description: m.description.clone(), previews: false, note: false, ok: None };
    let (id, params) = open_hosted_with(app, GENERATE_KIND, plugin.id(), m.clone(), look, values)?;
    app.plugin_dialog = Some(PluginDialog { id, kind: Opened::Generate { plugin: plugin.id().to_string(), edit, params } });
    Ok(())
}

/// `ui.editGeneratedArt {id?}`: opens the dialog of generated art object `id` (else the one
/// selected) with the parameters it was drawn with, for the parameters its plug-in still declares.
/// An object whose plug-in isn't installed, or whose saved fields are damaged, is an error that says
/// so, and its art stays as it is.
pub fn open_edit(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut q = json!({});
    if let Some(id) = p.get("id").filter(|v| !v.is_null()) {
        q["id"] = id.clone();
    }
    let info = app.run("plugin.generatedInfo", q)?;
    let id = info.get("id").cloned().unwrap_or(Value::Null);
    let installed = info.get("installed") == Some(&Value::Bool(true));
    let Some(plugin) = info.get("plugin").and_then(Value::as_str).and_then(registry::get).filter(|_| installed) else {
        // The engine's error says which plug-in is missing or that the saved fields are damaged.
        return app.run("plugin.editGenerated", json!({"id": id, "params": {}}));
    };
    let saved = info.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
    let edit = EditTarget { object: id, doc: app.session.active().map(|st| st.uid) };
    open_generate(app, &plugin, Some(edit), &saved)?;
    Ok(Value::Null)
}

/// Is the selection one object, a generated art object, which Edit Generated Art… edits?
pub fn one_generated_selected(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| match st.selection.objects.as_slice() {
        [id] => st.doc.node(*id).is_some_and(|n| n.extras().generated.is_some()),
        _ => false,
    })
}

/// OK of a generator dialog: draws new generated art (`plugin.generate`), or draws the object of
/// `edit` again (`plugin.editGenerated`), as one undo step. When another document has come to the
/// front since Edit Generated Art… opened the dialog, it draws nothing and says so.
fn generate_with(app: &mut VectorcraftApp, plugin: String, edit: Option<EditTarget>, values: Map<String, Value>) -> Result<Value, String> {
    match edit {
        Some(e) if e.doc != app.session.active().map(|st| st.uid) => Err(EDIT_DOC_GONE.into()),
        Some(e) => app.run("plugin.editGenerated", json!({"id": e.object, "params": values})),
        None => app.run(GENERATE, json!({"id": plugin, "params": values})),
    }
}

/// The key of an exporter's parameters dialog, after Export As picked the file.
pub const EXPORT_KIND: &str = "pluginExport";

/// The key of an importer's parameters dialog, after Open With picked the file.
pub const IMPORT_KIND: &str = "pluginImport";

/// Export As with exporter `plugin`, which has parameters: its rows at their defaults, and
/// `export`, the export's own parameters (`format`, `path`, …), kept for Export.
pub fn open_export(app: &mut VectorcraftApp, plugin: &Plugin, export: Value) {
    let m = plugin.manifest();
    let look = Look {
        title: m.name.trim_end_matches('…').to_string(),
        description: m.description.clone(),
        previews: false,
        note: false,
        ok: Some(tl!("Export")),
    };
    match open_hosted(app, EXPORT_KIND, plugin.id(), m.clone(), look) {
        Ok((id, params)) => app.plugin_dialog = Some(PluginDialog { id, kind: Opened::Export { plugin: plugin.id().to_string(), export, params } }),
        Err(e) => app.status(e),
    }
}

/// Open With with importer `plugin`, which has parameters: its rows at their defaults, and
/// `open`, Open With's own parameters (`path`, `importer`, and `picked` for a file from the
/// picker, which is read off the UI thread), kept for Open.
pub fn open_import(app: &mut VectorcraftApp, plugin: &Plugin, open: Value) {
    let m = plugin.manifest();
    let look = Look {
        title: m.name.trim_end_matches('…').to_string(),
        description: m.description.clone(),
        previews: false,
        note: false,
        ok: Some(tl!("Open")),
    };
    match open_hosted(app, IMPORT_KIND, plugin.id(), m.clone(), look) {
        Ok((id, params)) => app.plugin_dialog = Some(PluginDialog { id, kind: Opened::Import { plugin: plugin.id().to_string(), open, params } }),
        Err(e) => app.status(e),
    }
}

/// Writes the file with the export's parameters and the plug-in's, as `params`.
fn export_with(app: &mut VectorcraftApp, export: Value, values: Map<String, Value>) -> Result<Value, String> {
    let mut p = Some(export).filter(Value::is_object).ok_or("no format to export to")?;
    p["params"] = Value::Object(values);
    let format = p.get("format").and_then(Value::as_str).map(str::to_string);
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    crate::io::export(app, format.as_deref(), path, &p)
}

/// Opens the file with Open With's parameters and the plug-in's, as `params`.
fn import_with(app: &mut VectorcraftApp, open: Value, values: Map<String, Value>) -> Result<Value, String> {
    let mut p = Some(open).filter(Value::is_object).ok_or("no file to open")?;
    p["params"] = Value::Object(values);
    let picked = p["picked"] == true;
    crate::io::open_with_on(app, &p, picked && crate::opening::available())
}

/// OK of a filter dialog: a filter keeps its previewed run as one undo step; a heavy one, which has
/// no preview, and a light one that runs in the background ([`in_background`]) with Preview off or a
/// failed preview, start in the background ([`run`]). A preview still shown when Preview was just
/// turned off, or one that failed, is rolled back first.
fn confirm_filter(app: &mut VectorcraftApp, kind: Opened, values: Map<String, Value>) -> Result<Value, String> {
    let Opened::Filter { plugin, command: own, preview, light, previewing, refused, .. } = kind else { return Ok(Value::Null) };
    let (cmd, p) = command(&plugin, own.as_deref(), values);
    let kept = light && previewing && app.session.in_interaction() && !refused;
    match registry::get(&plugin) {
        Some(found) if own.is_none() && in_background(found.manifest()) && !kept => {
            preview.cancel(app);
            run(app, &found, p)
        }
        _ => preview.commit(app, &cmd, p),
    }
}

/// Once per frame, after the shell pass: the plug-in install answer, then the open plug-in dialog,
/// which previews with its values, closes when its plug-in is gone, and acts on its answer.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    pump_install(app);
    pump_dialog(app, ctx);
}

fn pump_dialog(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut open) = app.plugin_dialog.take() else { return };
    let Some(dialogs) = app.dialogs() else {
        app.plugin_dialog = Some(open);
        return;
    };
    let id = open.id;
    let params = Arc::clone(open.kind.params());
    let live = dialogs.with(|host| {
        let d = host.get_mut_by_id::<HostedParamDialog>(id)?;
        Some((d.take_help_link(), d.is_previewing(), form::json_of(&params, d.fields())))
    });
    if let Some((help, on, values)) = live {
        if let Some(link) = help {
            form::open_help(app, &link);
        }
        if open.kind.gone() {
            dialogs.with(|host| host.cancel_by_id(ctx, id));
            if let Opened::Filter { preview, .. } = open.kind {
                preview.cancel(app);
            }
            return;
        }
        if let Opened::Filter { plugin, command: own, label, preview, previewing, refused, light, .. } = &mut open.kind {
            *previewing = on;
            if *light {
                let (cmd, p) = command(plugin, own.as_deref(), values);
                if let Some(answer) = preview.run(app, on, label, &cmd, &p) {
                    *refused = answer.is_err();
                }
            }
        }
        app.plugin_dialog = Some(open);
        return;
    }
    // The dialog is closed: answered, or cancelled from outside.
    let answer = dialogs.with(|host| host.take_by_id::<HostedParamDialog>(id));
    let Some(DialogResult::Ok(result)) = answer else {
        if let Opened::Filter { preview, .. } = open.kind {
            preview.cancel(app);
        }
        return;
    };
    let values = form::json_of(&params, &result.fields);
    // The dialog is already closed, so a failure goes to the status line.
    let done = match open.kind {
        Opened::Filter { .. } => confirm_filter(app, open.kind, values),
        Opened::Generate { plugin, edit, .. } => generate_with(app, plugin, edit, values),
        Opened::Export { export, .. } => export_with(app, export, values),
        Opened::Import { open, .. } => import_with(app, open, values),
    };
    if let Err(e) = done {
        app.status(e);
    }
}

/// The key of the approval of a plug-in install: the prompt's, and what [`ask_install`] answers.
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
    /// The question shown for it, once [`ask_install`] opened it: only its answer counts.
    pub question: Option<DialogId>,
}

/// How long, in seconds, the approval's question stays on screen in the focused window, on every
/// frame and unchanged, before its OK takes a click, Enter or Space.
pub(crate) const APPROVAL_DELAY: f64 = 0.5;

/// What the approval says when automation asked for the install.
fn automation_asked_note() -> &'static str {
    tl!("Automation asked to install this plug-in. To install it, choose Object › Plug-ins › Install Plug-in… yourself.")
}

/// The approval of a plug-in install: the question, the lines under it, OK, which waits
/// [`APPROVAL_DELAY`] ([`FooterButton::armed_after`]), and Cancel, which never waits. When automation
/// asked, a note says so and OK stays disabled. It takes no programmatic OK and sets no fields.
pub(crate) struct InstallPrompt {
    /// A new id for every question, so the OK's arming starts over for each.
    id: DialogId,
    message: String,
    detail: String,
    automated: bool,
    /// Automation's injected input pressed OK since [`pump_install`] last looked: the question stays open.
    refused: bool,
}

impl InstallPrompt {
    fn new(pending: &PendingInstall) -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        InstallPrompt {
            id: dialog_id((INSTALL_KIND, SERIAL.fetch_add(1, Ordering::Relaxed))),
            message: pending.message.clone(),
            detail: pending.detail.clone(),
            automated: pending.origin == InstallOrigin::Automation,
            refused: false,
        }
    }
}

impl StackDialog for InstallPrompt {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(INSTALL_KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let mut prompt = Prompt::new(tl!("Install Plug-in"), self.message.clone()).paragraph(self.detail.clone());
        if self.automated {
            prompt = prompt.paragraph(automation_asked_note());
        }
        // An OK pressed by the pass's injected input is refused and leaves the question open.
        let injected = WindowHost::of(ui.ctx()).injected_pass();
        // `Other`: Enter answers only an accepting or destructive answer, and OK must not take it unfocused.
        let shown = prompt
            .answers([
                FooterButton::new(tl!("OK"), DialogRole::Other).enabled(!self.automated).armed_after(APPROVAL_DELAY).stays_open(injected),
                FooterButton::new(tl!("Cancel"), DialogRole::Cancel),
            ])
            .show(ui);
        match shown.picked {
            Some(0) if injected => {
                self.refused = true;
                None
            }
            Some(0) => Some(DialogResult::Ok(())),
            Some(1) => Some(DialogResult::Cancel),
            _ => None,
        }
    }
}

/// Asks a person whether to install what `pending` describes, its question not yet shown, even when
/// it reads as the one it replaces. → `{pending}`, and `approval: "personMustStartInstall"` when
/// automation asked.
pub(crate) fn ask_install(app: &mut VectorcraftApp, mut pending: PendingInstall) -> Value {
    let prompt = InstallPrompt::new(&pending);
    let automated = prompt.automated;
    pending.question = Some(prompt.id);
    app.pending_plugin_install = Some(pending);
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| host.open(prompt));
    }
    if automated {
        return json!({ "pending": INSTALL_KIND, "approval": PERSON_MUST_START_INSTALL });
    }
    json!({ "pending": INSTALL_KIND })
}

/// What the status line says when automation pressed the approval's OK.
const REFUSED: &str = "Only the person at the keyboard can approve installing a plug-in";

/// The answer to the install question: Cancel drops it; OK installs the module the question showed,
/// when a person pressed it. An OK automation's input pressed leaves the question open and says why.
fn pump_install(app: &mut VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    let question = app.pending_plugin_install.as_ref().and_then(|p| p.question);
    let (answer, refused) = dialogs.with(|host| {
        let refused = question.and_then(|id| host.get_mut_by_id::<InstallPrompt>(id)).is_some_and(|p| std::mem::take(&mut p.refused));
        let answer = question.and_then(|id| host.take_by_id::<InstallPrompt>(id));
        // A question a newer one replaced was answered Cancel: that answer is not the pending install's.
        while host.take::<InstallPrompt>().is_some() {}
        (answer, refused)
    });
    if refused {
        app.status(REFUSED.to_string());
    }
    let Some(answer) = answer else { return };
    match answer.ok() {
        Some(()) => approve(app),
        None => app.pending_plugin_install = None,
    }
}

/// A person's OK of the install question. A question automation asked has no OK that installs, and
/// a control request can't press one: either asks again, a new question that waits its delay.
fn approve(app: &mut VectorcraftApp) {
    let Some(pending) = app.pending_plugin_install.take() else { return };
    if pending.origin == InstallOrigin::Automation {
        ask_install(app, pending);
        return;
    }
    if app.automation_is_acting() {
        app.status(REFUSED.to_string());
        ask_install(app, pending);
        return;
    }
    app.session.approve_plugin_install(pending.approval);
    let r = app.run("plugin.install", pending.params);
    app.session.clear_plugin_install_approval();
    match r {
        Ok(r) => crate::io::report_installed(app, &r),
        Err(e) => app.status(e),
    }
}
