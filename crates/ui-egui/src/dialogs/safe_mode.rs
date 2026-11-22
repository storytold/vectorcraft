//! Safe Mode in the app: the choice after a launch that stopped while a plug-in was loading
//! ([`ask_about_stopped`]: Disable and Continue, Start Normally, or Start in Safe Mode), and Help ›
//! Safe Mode ([`open`]), which lists the disabled plug-ins with Enable on Re-launch. Safe Mode
//! itself, the disabled list and the launch marker are `vectorcraft_plugins::safe_mode`'s.

use serde_json::{Value, json};
use vectorcraft_plugins::safe_mode::{self, Item};

use super::DialogSpec;
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;

/// Dialog kind of the choice after a launch that stopped. Fields: `items` (`[{id, file, name,
/// label}]`, the plug-ins that were loading, oldest first), `item` (the first of them) and
/// `normal` (Start Normally instead of Disable and Continue). Cancel starts in Safe Mode.
pub const STOPPED: &str = "pluginStopped";

/// Dialog kind of Help › Safe Mode. Fields: `items` (`[{id, file, name, label, enable}]`, the
/// disabled plug-ins, `enable` being Enable on Re-launch).
pub const KIND: &str = "safeMode";

/// Before any plug-in loads (desktop): starts this launch's marker in `marker_dir` (none: no
/// marker), and turns Safe Mode on for `--safe-mode` (`flag`), or until the user chooses after a
/// launch that stopped while plug-ins were loading → those plug-ins, for [`after_start`]. With
/// `flag` nothing is asked, and the plug-ins stay pending (`plugin.safeMode`'s `pendingItems`): the
/// next launch asks about them.
#[cfg(not(target_arch = "wasm32"))]
pub fn begin_launch(marker_dir: Option<&std::path::Path>, flag: bool) -> Vec<Item> {
    let stopped = marker_dir.map(safe_mode::begin_launch).unwrap_or_default();
    if flag {
        safe_mode::set_active(Some(safe_mode::Source::Flag));
        return Vec::new();
    }
    if !stopped.is_empty() {
        safe_mode::set_active(Some(safe_mode::Source::Startup));
    }
    stopped
}

/// The app is quitting normally: its launch marker goes.
#[cfg(not(target_arch = "wasm32"))]
pub fn end_launch() {
    safe_mode::end_launch();
}

/// Once the app is made: the choice about `stopped` ([`begin_launch`]), or in Safe Mode a status
/// line saying so.
pub fn after_start(app: &mut VectorcraftApp, stopped: Vec<Item>) {
    if !stopped.is_empty() {
        ask_about_stopped(app, &stopped);
    } else if safe_mode::active() {
        app.status(vectorcraft_plugins::Error::SafeMode.to_string());
    }
}

/// Ask what to do about `items`, the plug-ins the last launch was loading when it stopped. Safe
/// Mode is on until the user chooses. A dialog that takes the choice's place leaves it waiting: it
/// opens again once no dialog is open ([`reopen`]).
pub fn ask_about_stopped(app: &mut VectorcraftApp, items: &[Item]) {
    let items: Vec<Value> = items.iter().map(Item::to_json).collect();
    let fields = json!({ "item": items.first(), "items": items });
    app.ui.stopped_plugin = Some(fields.clone());
    app.ui.dialog = Some(Dialog::new(STOPPED, fields));
}

/// With no dialog open, the choice about a stopped plug-in that another dialog replaced opens again.
pub(super) fn reopen(app: &mut VectorcraftApp) {
    if app.ui.dialog.is_none()
        && let Some(fields) = app.ui.stopped_plugin.clone()
    {
        app.ui.dialog = Some(Dialog::new(STOPPED, fields));
    }
}

/// The choice's Cancel, Start in Safe Mode: Safe Mode stays on for this launch, and the plug-in
/// stays pending for the next one.
pub(super) fn cancel_stopped(app: &mut VectorcraftApp) {
    app.ui.stopped_plugin = None;
}

/// The choice about a stopped plug-in is waiting for the user (shown, or replaced by another
/// dialog): Safe Mode is on, and nothing has loaded yet.
pub fn choosing(app: &VectorcraftApp) -> bool {
    app.ui.stopped_plugin.is_some()
}

/// Safe Mode is on for the rest of this launch: `--safe-mode`, or Start in Safe Mode chosen.
pub fn staying_on(app: &VectorcraftApp) -> bool {
    safe_mode::active() && !choosing(app)
}

/// The item a dialog field holds.
fn item_of(v: &Value) -> Item {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Item { id: s("id"), file: s("file"), name: s("name") }
}

/// The items the choice's fields name: its `items`, else its `item`.
fn stopped_items(fields: &serde_json::Map<String, Value>) -> Vec<Item> {
    match fields.get("items").and_then(Value::as_array).filter(|i| !i.is_empty()) {
        Some(items) => items.iter().map(item_of).collect(),
        None => fields.get("item").filter(|i| i.is_object()).map(item_of).into_iter().collect(),
    }
}

/// How many plug-ins the waiting choice names.
fn stopped_count(app: &VectorcraftApp) -> usize {
    app.ui.stopped_plugin.as_ref().and_then(Value::as_object).map_or(0, |f| stopped_items(f).len())
}

/// The choice's Disable and Continue (OK) or Start Normally: Safe Mode ends and the plug-ins load,
/// without the stopped ones for Disable and Continue (kept off for later launches too).
fn confirm_stopped(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    app.ui.stopped_plugin = None;
    // When a disabled plug-in can't be saved for the next launch, that launch asks again. Every
    // listed plug-in is turned off for this launch, whether or not the one before it was saved.
    let saved = d.bool("normal") || {
        let mut all = true;
        for item in stopped_items(&d.fields) {
            all &= safe_mode::disable(item);
        }
        all && app.session.store_disabled_plugins()
    };
    if saved {
        safe_mode::forget_previous();
    } else {
        not_saved(app);
    }
    safe_mode::set_active(None);
    vectorcraft_engine::cmd::plugin::load_prefs_folder(&mut app.session);
    Ok(Value::Null)
}

/// Help › Safe Mode: the disabled plug-ins, none ticked for Enable on Re-launch. A plug-in disabled
/// by its id and found under several files has an entry for each file; it is listed once, and
/// Enable on Re-launch turns on every entry with its id.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let disabled = safe_mode::disabled();
    let items: Vec<Value> = disabled
        .iter()
        .enumerate()
        .filter(|(k, i)| i.id.is_none() || !disabled[..*k].iter().any(|e| e.id == i.id))
        .map(|(_, i)| {
            let mut v = i.to_json();
            v["enable"] = json!(false);
            v
        })
        .collect();
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "items": items })));
    Ok(Value::Null)
}

/// Help › Safe Mode's OK: each ticked plug-in loads again from the next launch
/// (`plugin.setEnabled`).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    let mut errors = vec![];
    let mut unsaved = false;
    for v in d.fields.get("items").and_then(Value::as_array).into_iter().flatten().filter(|v| v["enable"] == true) {
        let p = match (v["id"].as_str(), v["file"].as_str()) {
            (Some(id), _) => json!({"id": id, "enabled": true}),
            (None, Some(file)) => json!({"file": file, "enabled": true}),
            (None, None) => continue,
        };
        match app.run("plugin.setEnabled", p) {
            Ok(r) => unsaved |= r["saved"] == false,
            Err(e) => errors.push(e),
        }
    }
    if unsaved {
        not_saved(app);
    }
    if errors.is_empty() { Ok(Value::Null) } else { Err(errors.join("; ")) }
}

/// The disabled plug-ins couldn't be written to the plug-in folder: the next launch doesn't have
/// the change, which goes with the next change that saves.
fn not_saved(app: &mut VectorcraftApp) {
    app.status("Couldn't save the disabled plug-ins for the next launch");
}

pub(super) const STOPPED_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Safe Mode").into(),
    body: |_, ui, d| {
        let t = Tokens::get(ui.ctx());
        let items = stopped_items(&d.fields);
        if let [item] = items.as_slice() {
            let text = crate::i18n::fmt(
                tl!("The last launch of VectorCraft stopped while the plug-in {name} was loading. It may not be what stopped it."),
                &[("name", &item.label())],
            );
            ui.label(egui::RichText::new(text).color(t.text));
            return false;
        }
        ui.label(
            egui::RichText::new(tl!(
                "The last launch of VectorCraft stopped while these plug-ins were loading. Any one of them, or none, may be what stopped it."
            ))
            .color(t.text),
        );
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
            for item in &items {
                ui.label(egui::RichText::new(item.label()).color(t.text));
            }
        });
        false
    },
    confirm: confirm_stopped,
    ok: Some("Disable and Continue"),
    ok_label: Some(|app| if stopped_count(app) > 1 { "Disable Listed Plug-ins and Continue" } else { "Disable and Continue" }),
    cancel: Some("Start in Safe Mode"),
    alternate: Some(("Start Normally", "normal")),
    min_width: 380.0,
    max_width: Some(520.0),
    ..DialogSpec::FORM
};

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Safe Mode").into(),
    body: |_, ui, d| {
        let t = Tokens::get(ui.ctx());
        if safe_mode::active() {
            ui.label(egui::RichText::new(tl!("VectorCraft is in Safe Mode: no plug-ins load or run until it starts normally.")).color(t.text));
            ui.add_space(8.0);
        }
        ui.label(egui::RichText::new(tl!("Disabled plug-ins")).color(t.text_dim));
        let Some(items) = d.fields.get_mut("items").and_then(Value::as_array_mut).filter(|i| !i.is_empty()) else {
            ui.label(egui::RichText::new(tl!("No plug-ins are disabled.")).color(t.text_dim));
            return false;
        };
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
            for (k, v) in items.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    // A name made safe to show: control and reordering characters removed, bounded.
                    let label = safe_mode::display_name(v["label"].as_str().unwrap_or_default());
                    ui.label(egui::RichText::new(label).color(t.text));
                    let mut on = v["enable"] == true;
                    if ui.push_id(k, |ui| ui.checkbox(&mut on, tl!("Enable on Re-launch"))).inner.changed() {
                        v["enable"] = json!(on);
                    }
                });
            }
        });
        false
    },
    confirm,
    min_width: 380.0,
    max_width: Some(520.0),
    ..DialogSpec::FORM
};
