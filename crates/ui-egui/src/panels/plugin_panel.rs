//! Panel plug-ins' panels (`"kind": "panel"`, docs/plugins.md › Panels): each installed panel
//! plug-in has a panel of its own, `plugin:<plug-in id>` ([`panel_id`]), that floats, tabs, docks
//! and closes as any panel of the shared docking system. Window › Plug-ins lists them, and
//! `window.pluginPanel {id}` opens one, or brings it forward when it is open.
//!
//! The panel draws what the plug-in answered last (`plugin.panel.event`): its `items`, top to
//! bottom ([`item`]). An item of a type this host doesn't know is left out. Drawn for the first time
//! since it opened, the panel asks the plug-in what it shows (an `open` event); a button clicked, a
//! checkbox or dropdown changed, a field's text or number committed and a file picked with a file
//! field ([`pick_file`]) are events too, sent with the values of all the panel's fields, and the
//! answer is drawn from then on. A plug-in that asks for selection events is also sent one when the
//! selection changed while its panel shows ([`selection_event`]). A failed event leaves the panel as
//! it was and says why under it. Plug-in text is shown as it is, never translated.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

use egui::{RichText, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_plugins::panel::MAX_FILE;
use vectorcraft_plugins::{Kind, PanelEvent, Plugin, registry};

use crate::VectorcraftApp;
use crate::menus::Item;
use crate::state::UiState;
use crate::theme::Tokens;
use crate::widgets;

/// What every plug-in panel's id starts with.
pub const PREFIX: &str = "plugin:";

/// Window › Plug-ins while no panel plug-in is installed.
pub const NO_PANELS: &str = "No panel plug-ins installed";

/// The most plug-in panels one session opens: their ids are interned ([`intern`]).
const MAX_PANELS: usize = 256;

/// Most items one panel draws, rows' and groups' included.
const MAX_ITEMS: usize = 500;

/// How deep rows and groups nest.
const MAX_DEPTH: usize = 6;

/// The shortest time between two selection events to one panel, in seconds.
const SELECTION_INTERVAL: f64 = 0.25;

/// What a plug-in's panel shows.
#[derive(Clone, Debug, Default)]
pub struct PluginPanel {
    /// The `panel` the plug-in answered last (`null` until it answered).
    pub panel: Value,
    /// The fields' values, by field id: as the plug-in last gave them, then as they were set.
    pub values: Map<String, Value>,
    /// Why the last event failed.
    pub error: Option<String>,
    /// The selection the plug-in was given with its last event ([`selection_key`]).
    pub seen: Option<u64>,
    /// When it was last sent a selection event (egui's input time, in seconds).
    pub selection_at: Option<f64>,
}

/// The id of plug-in `plugin`'s panel (`plugin:org.example.shapes`).
pub fn panel_id(plugin: &str) -> String {
    format!("{PREFIX}{plugin}")
}

/// The plug-in id of plug-in panel `panel`.
pub(crate) fn parse(panel: &str) -> Option<&str> {
    let id = panel.strip_prefix(PREFIX)?;
    let valid = !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    valid.then_some(id)
}

/// Plug-in panel `panel` as the `&'static str` the docking code keeps panel ids as
/// ([`crate::menus::normalize_panel`]); `None` when it isn't a plug-in panel's id. Each id is
/// leaked once, at most [`MAX_PANELS`] of them, as library panels' are.
pub(crate) fn intern(panel: &str) -> Option<&'static str> {
    parse(panel)?;
    static IDS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut ids = IDS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(id) = ids.iter().find(|id| **id == panel) {
        return Some(id);
    }
    if ids.len() >= MAX_PANELS {
        return None;
    }
    let id: &'static str = Box::leak(panel.into());
    ids.push(id);
    Some(id)
}

/// The installed panel plug-in `id`.
fn installed(id: &str) -> Option<Arc<Plugin>> {
    registry::get(id).filter(|p| p.manifest().kind == Kind::Panel)
}

/// The plug-in's name as a menu or tab shows it.
fn name(p: &Plugin) -> &str {
    p.manifest().name.trim_end_matches('…')
}

/// Plug-in panel `panel`'s tab label: its plug-in's name, else its id once the plug-in is gone.
pub(crate) fn tab_label(panel: &str) -> Option<String> {
    let id = parse(panel)?;
    Some(installed(id).map_or_else(|| id.to_string(), |p| name(&p).to_string()))
}

/// Window › Plug-ins: the installed panel plug-ins.
pub fn window_menu() -> Vec<Item> {
    let mut items: Vec<Item> = registry::list()
        .iter()
        .filter(|p| p.manifest().kind == Kind::Panel)
        .map(|p| Item::Cmd(registry::intern(name(p)), "window.pluginPanel", json!({ "id": p.id() })))
        .collect();
    if items.is_empty() {
        items.push(Item::Header(NO_PANELS));
    }
    items
}

/// Is panel `panel` open (in the dock or floating)?
fn is_open(ui: &UiState, panel: &str) -> bool {
    crate::panel_docking::layout_of(ui).contains(&panel.to_string())
}

/// The open plug-in panels.
fn open_panels(ui: &UiState) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for p in ui.docking.iter().flat_map(|l| l.panels()) {
        if p.starts_with(PREFIX) && !out.contains(p) {
            out.push(p.clone());
        }
    }
    out
}

/// `window.pluginPanel {id}`: open panel plug-in `id`'s panel, floating, or bring it forward when it
/// is open. → `{open: id, panel}`
pub(crate) fn open_command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_str).ok_or("missing `id`: a panel plug-in (see plugin.list)")?;
    let plugin = registry::get(id).ok_or_else(|| format!("no plug-in `{id}` (see plugin.list)"))?;
    if plugin.manifest().kind != Kind::Panel {
        return Err(format!("`{id}` is a {}, not a panel", plugin.manifest().kind.name()));
    }
    let panel = panel_id(id);
    let registered = intern(&panel).ok_or("too many plug-in panels were opened in this session")?;
    if is_open(&app.ui, &panel) {
        app.run("window.panel.activate", json!({ "panel": panel }))?;
    } else {
        float(app, registered)?;
    }
    Ok(json!({ "open": id, "panel": panel }))
}

/// Float plug-in panel `panel`. Opened again, it asks its plug-in afresh what it shows.
pub(crate) fn float(app: &mut VectorcraftApp, panel: &str) -> Result<Value, String> {
    if let Some(id) = parse(panel) {
        app.ui.plugin_panels.remove(id);
    }
    app.run("window.panel.float", json!({ "panel": panel, "height": 320 }))
}

/// Per frame, before the panels are drawn: close the panels of plug-ins that are gone.
pub fn tidy(app: &mut VectorcraftApp) {
    for panel in open_panels(&app.ui) {
        if parse(&panel).is_none_or(|id| installed(id).is_none()) {
            if let Err(e) = app.run("window.panel.close", json!({ "panel": panel })) {
                app.status(e);
            }
            if let Some(id) = parse(&panel) {
                app.ui.plugin_panels.remove(id);
            }
        }
    }
}

/// Sends `event` to panel plug-in `id` with its fields' values, and keeps the answer, or why it
/// failed. The plug-in was given the selection as it was before the commands it answered with ran,
/// which it is then sent a selection event for, when it asks for them and they changed it.
fn send(app: &mut VectorcraftApp, id: &str, event: Value) {
    let values = app.ui.plugin_panels.get(id).map_or_else(|| json!({}), |p| Value::Object(p.values.clone()));
    let seen = selection_key(app);
    let r = app.run("plugin.panel.event", json!({ "id": id, "event": event, "values": values }));
    let view = app.ui.plugin_panels.entry(id.to_string()).or_default();
    // Also when it failed: the same selection isn't sent again and again.
    view.seen = Some(seen);
    match r {
        Ok(v) => {
            view.panel = v.get("panel").cloned().unwrap_or(Value::Null);
            given_values(&view.panel, &mut view.values, 0);
            view.error = None;
        }
        Err(e) => view.error = Some(e),
    }
}

/// What a selection event tells apart: the active document and the objects selected in it, in
/// order (not their anchors, nor edits to them).
fn selection_key(app: &VectorcraftApp) -> u64 {
    let mut h = DefaultHasher::new();
    match app.session.active() {
        Some(d) => (d.uid, &d.selection.objects).hash(&mut h),
        None => 0u8.hash(&mut h),
    }
    h.finish()
}

/// Panel plug-in `id`, which asks for selection events, while its panel shows: sent one when the
/// selection changed since its last event ([`selection_key`]). Not while a drag or typing goes on
/// (once it ends), and at most every [`SELECTION_INTERVAL`] seconds: a change within that is sent
/// once it has passed, a frame being asked for then.
fn selection_event(app: &mut VectorcraftApp, ctx: &egui::Context, id: &str) {
    if app.session.in_interaction() {
        return;
    }
    let Some(view) = app.ui.plugin_panels.get(id) else { return };
    if view.seen == Some(selection_key(app)) {
        return;
    }
    let now = ctx.input(|i| i.time);
    if let Some(wait) = view.selection_at.map(|at| at + SELECTION_INTERVAL - now).filter(|w| *w > 0.0) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        return;
    }
    if let Some(v) = app.ui.plugin_panels.get_mut(id) {
        v.selection_at = Some(now);
    }
    send(app, id, json!({ "type": "selection" }));
}

/// Takes the values the fields of `panel` (an answer, or a row or group in it) are given.
fn given_values(panel: &Value, values: &mut Map<String, Value>, depth: usize) {
    let Some(items) = panel.get("items").and_then(Value::as_array) else { return };
    for item in items.iter().take(MAX_ITEMS) {
        if let (Some(id), Some(v)) = (item.get("id").and_then(Value::as_str), item.get("value")) {
            values.insert(id.to_string(), v.clone());
        }
        if depth < MAX_DEPTH {
            given_values(item, values, depth + 1);
        }
    }
}

/// Plug-in panel `panel`'s contents (the dock or its floating group draws its tab).
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut Ui, panel: &str) {
    let Some(id) = parse(panel).map(str::to_string) else { return };
    let Some(plugin) = installed(&id) else {
        widgets::dim_label(ui, tl!("This plug-in is no longer installed."));
        return;
    };
    if !app.ui.plugin_panels.contains_key(&id) {
        send(app, &id, json!({ "type": "open" }));
    }
    let Some(view) = app.ui.plugin_panels.get(&id).cloned() else { return };
    let mut values = view.values;
    let mut event = None;
    let mut pick = None;
    let mut budget = MAX_ITEMS;
    // The web app shows no open dialog that answers with a file's path.
    let can_pick = crate::picks::can(app, &crate::picks::PickRequest::Open(crate::FilePick::default()));
    if let Some(items) = view.panel.get("items").and_then(Value::as_array) {
        let mut f = Fields { panel: &id, values: &mut values, event: &mut event, pick: &mut pick, can_pick, budget: &mut budget };
        f.items(ui, items, 0);
    }
    if let Some(e) = &view.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(Tokens::get(ui.ctx()).error).size(12.0));
    }
    if let Some(v) = app.ui.plugin_panels.get_mut(&id) {
        v.values = values;
    }
    if let Some(event) = event {
        send(app, &id, event);
    }
    if let Some((target, extensions)) = pick {
        pick_file(app, &id, target, extensions);
    }
    if plugin.manifest().events.contains(&PanelEvent::Selection) {
        selection_event(app, ui.ctx(), &id);
    }
}

/// File field `target` of panel plug-in `id` clicked: an open dialog for files with `extensions`
/// (any file without them), and the file picked sent as a `file` event with its name and bytes,
/// never its folder. Shown off the UI thread, the dialog's answer runs this again
/// ([`crate::picks::button`]). A file that can't be read, or is larger than a panel takes, sends
/// nothing and is said under the panel.
fn pick_file(app: &mut VectorcraftApp, id: &str, target: String, extensions: Vec<String>) {
    let id = id.to_string();
    crate::picks::button(app, move |app| {
        let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
        let filters = if exts.is_empty() { vec![] } else { vec![crate::FilePick::filter(&exts.join(", ").to_uppercase(), &exts)] };
        let Some(path) = crate::picks::open(app, &crate::FilePick { filters, ..Default::default() }) else { return };
        match read_picked(app, &path) {
            Ok((name, bytes)) => {
                let event = json!({ "type": "file", "target": target, "name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes) });
                send(app, &id, event);
            }
            Err(e) => app.ui.plugin_panels.entry(id.clone()).or_default().error = Some(e),
        }
    });
}

// The message below names the size.
const _: () = assert!(MAX_FILE == 4 << 20);

/// The name, without its folder, and the bytes of the file at `path`, picked for a panel.
fn read_picked(app: &VectorcraftApp, path: &str) -> Result<(String, Vec<u8>), String> {
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| format!("{path}: not a file"))?;
    let read = app.services.read.as_ref().ok_or("no file reader")?;
    let bytes = read(path)?;
    if bytes.len() > MAX_FILE {
        return Err(tl!("The file is larger than 4 MiB, the most a plug-in panel is given.").to_string());
    }
    Ok((name, bytes))
}

/// The file extensions a file field's `extensions` list: lower-case letters and digits, 1-16 of
/// them each, at most 16 extensions; others are left out.
fn extensions(it: &Value) -> Vec<String> {
    let valid = |e: &&str| (1..=16).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let list = it.get("extensions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    list.iter().filter_map(Value::as_str).filter(valid).take(16).map(str::to_string).collect()
}

/// What drawing a panel's items collects: the values set, and the event to send.
struct Fields<'a> {
    panel: &'a str,
    values: &'a mut Map<String, Value>,
    event: &'a mut Option<Value>,
    /// The file field clicked: its id and the extensions it takes.
    pick: &'a mut Option<(String, Vec<String>)>,
    /// Whether file fields can show an open dialog here.
    can_pick: bool,
    /// How many more items may be drawn.
    budget: &'a mut usize,
}

impl Fields<'_> {
    fn items(&mut self, ui: &mut Ui, items: &[Value], depth: usize) {
        for it in items {
            if *self.budget == 0 {
                return;
            }
            *self.budget -= 1;
            self.item(ui, it, depth);
        }
    }

    /// One item: `text`, `heading`, `separator`, `button`, `file` (a button that opens a file
    /// dialog for its `extensions`; off where the app shows none, as on the web), `string` (a text
    /// field, `multiline` for several lines), `number` and `int` (`min`, `max`, `step`, `unit`),
    /// `bool` (a checkbox), `choice` (a dropdown of `options`), `row` (its `items` side by side) or
    /// `group` (its `items` under a `label` that folds, `open` at first unless it says `false`).
    fn item(&mut self, ui: &mut Ui, it: &Value, depth: usize) {
        let s = |k: &str| it.get(k).and_then(Value::as_str).unwrap_or_default();
        let id = s("id");
        let label = s("label");
        let t = Tokens::get(ui.ctx());
        match s("type") {
            "text" => {
                ui.label(RichText::new(s("text")).color(t.text).size(12.5));
            }
            "heading" => {
                ui.label(RichText::new(s("text")).color(t.text_strong).size(13.0).strong());
            }
            "separator" => widgets::divider(ui),
            "button" if !id.is_empty() => {
                let enabled = it.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                if ui.add_enabled_ui(enabled, |ui| widgets::flat_button_fit(ui, label, 60.0)).inner.clicked() {
                    *self.event = Some(json!({ "type": "click", "target": id }));
                }
            }
            "file" if !id.is_empty() => {
                let enabled = self.can_pick && it.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                if ui.add_enabled_ui(enabled, |ui| widgets::flat_button_fit(ui, label, 60.0)).inner.clicked() {
                    *self.pick = Some((id.to_string(), extensions(it)));
                }
            }
            "string" if !id.is_empty() => {
                let rows = if it.get("multiline").and_then(Value::as_bool).unwrap_or(false) { 4 } else { 1 };
                let now = self.values.get(id).and_then(Value::as_str).unwrap_or_default().to_string();
                field(ui, label, |ui| {
                    let width = ui.available_width().max(60.0);
                    if let Some(text) = widgets::text_field(ui, (self.panel, id), Some(&now), width, rows) {
                        self.set(id, json!(text));
                    }
                });
            }
            kind @ ("number" | "int") if !id.is_empty() => {
                let int = kind == "int";
                let num = |k: &str| it.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
                let (min, max) = (num("min").unwrap_or(-1e9), num("max").unwrap_or(1e9));
                let step = num("step").unwrap_or(1.0);
                let now = self.values.get(id).and_then(Value::as_f64).unwrap_or(0.0);
                field(ui, label, |ui| {
                    let decimals = if int { 0 } else { 2 };
                    if let Some(v) = widgets::spin_plain(ui, (self.panel, id), now, s("unit"), decimals, 72.0, step, min, &[]) {
                        let v = v.clamp(min, max.max(min));
                        self.set(id, if int { json!(v.round() as i64) } else { json!(v) });
                    }
                });
            }
            "bool" if !id.is_empty() => {
                let on = self.values.get(id).and_then(Value::as_bool).unwrap_or(false);
                if widgets::check_name(ui, label, on) {
                    self.set(id, json!(!on));
                }
            }
            "choice" if !id.is_empty() => {
                let options: Vec<&str> =
                    it.get("options").and_then(Value::as_array).map(|o| o.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                let now = self.values.get(id).and_then(Value::as_str).unwrap_or_default().to_string();
                field(ui, label, |ui| {
                    if let Some(i) = widgets::dropdown_names(ui, (self.panel, id), &now, &options, 140.0)
                        && let Some(o) = options.get(i)
                    {
                        self.set(id, json!(o));
                    }
                });
            }
            "row" if depth < MAX_DEPTH => {
                let items = it.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
                ui.horizontal(|ui| self.items(ui, &items, depth + 1));
            }
            "group" if depth < MAX_DEPTH => {
                let items = it.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
                let open = it.get("open").and_then(Value::as_bool).unwrap_or(true);
                egui::CollapsingHeader::new(RichText::new(label).color(t.text_strong).size(12.5))
                    .id_salt((self.panel, "group", id, label))
                    .default_open(open)
                    .show(ui, |ui| self.items(ui, &items, depth + 1));
            }
            _ => {}
        }
    }

    /// Field `id` was set to `value`: an event to send.
    fn set(&mut self, id: &str, value: Value) {
        self.values.insert(id.to_string(), value.clone());
        *self.event = Some(json!({ "type": "change", "target": id, "value": value }));
    }
}

/// A field row: its `label` (shown as it is), then what `add` draws.
fn field(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        if !label.is_empty() {
            widgets::field_label(ui, RichText::new(label).color(Tokens::get(ui.ctx()).text).size(12.5));
        }
        add(ui);
    });
}
