//! Safe Mode in the app: the choice after a launch that stopped while a plug-in was loading
//! ([`ask_about_stopped`]: Disable and Continue, Start Normally, or Start in Safe Mode), and Help ›
//! Safe Mode ([`open`]), which lists the disabled plug-ins with Enable on Re-launch. Safe Mode
//! itself, the disabled list and the launch marker are `vectorcraft_plugins::safe_mode`'s. Both
//! dialogs live on the dialog host; [`pump`] reads their answers.

use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogResult, PromptChoice, StandardPrompt, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use serde_json::{Value, json};
use vectorcraft_plugins::safe_mode::{self, Item};

use crate::VectorcraftApp;

/// Key of the choice after a launch that stopped. It holds `UiState::stopped_plugin` until
/// answered: `items` (`[{id, file, name, label}]`, the plug-ins that were loading, oldest first)
/// and `item` (the first of them). Cancel starts in Safe Mode.
pub const STOPPED: &str = "pluginStopped";

/// Key of Help › Safe Mode's dialog. It has a check box `enable-{k}` (Enable on Re-launch) and the
/// plug-in it stands for, `item-{k}`, for each disabled plug-in listed.
pub const KIND: &str = "safeMode";

/// The field holding the plug-in of row `k`.
fn item_key(k: usize) -> String {
    format!("item-{k}")
}

/// The field holding Enable on Re-launch of row `k`.
fn enable_key(k: usize) -> String {
    format!("enable-{k}")
}

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
/// Mode is on until the user chooses. The app holds the items; [`pump`] puts the question on the
/// dialog host once the host exists and no other dialog holds it, so a dialog that takes the
/// choice's place leaves it waiting.
pub fn ask_about_stopped(app: &mut VectorcraftApp, items: &[Item]) {
    let items: Vec<Value> = items.iter().map(Item::to_json).collect();
    app.ui.stopped_plugin = Some(json!({ "item": items.first(), "items": items }));
}

/// The choice's Cancel, Start in Safe Mode: Safe Mode stays on for this launch, and the plug-in
/// stays pending for the next one.
fn cancel_stopped(app: &mut VectorcraftApp) {
    app.ui.stopped_plugin = None;
}

/// The choice about a stopped plug-in is waiting for the user (shown, or held back by another
/// dialog): Safe Mode is on, and nothing has loaded yet.
pub fn choosing(app: &VectorcraftApp) -> bool {
    app.ui.stopped_plugin.is_some()
}

/// Safe Mode is on for the rest of this launch: `--safe-mode`, or Start in Safe Mode chosen.
pub fn staying_on(app: &VectorcraftApp) -> bool {
    safe_mode::active() && !choosing(app)
}

/// The item a held plug-in value names.
fn item_of(v: &Value) -> Item {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Item { id: s("id"), file: s("file"), name: s("name") }
}

/// The items the held choice names: its `items`, else its `item`.
fn stopped_items(fields: &Value) -> Vec<Item> {
    match fields.get("items").and_then(Value::as_array).filter(|i| !i.is_empty()) {
        Some(items) => items.iter().map(item_of).collect(),
        None => fields.get("item").filter(|i| i.is_object()).map(item_of).into_iter().collect(),
    }
}

/// The question about the stopped plug-ins the held choice `fields` names.
fn stopped_prompt(fields: &Value) -> StandardPrompt {
    let items = stopped_items(fields);
    let (text, proceed) = if let [item] = items.as_slice() {
        let text = crate::i18n::fmt(
            tl!("The last launch of VectorCraft stopped while the plug-in {name} was loading. It may not be what stopped it."),
            &[("name", &item.label())],
        );
        (text, tl!("Disable and Continue").to_string())
    } else {
        let mut text =
            tl!("The last launch of VectorCraft stopped while these plug-ins were loading. Any one of them, or none, may be what stopped it.")
                .to_string();
        text.push('\n');
        for item in &items {
            // A name made safe to show: control and reordering characters removed, bounded.
            text.push('\n');
            text.push_str(&safe_mode::display_name(&item.label()));
        }
        (text, tl!("Disable Listed Plug-ins and Continue").to_string())
    };
    StandardPrompt::custom(text)
        .title(tl!("Safe Mode").to_string())
        .button(PromptChoice::Proceed, proceed)
        .button(PromptChoice::Alternate, tl!("Start Normally").to_string())
        .button(PromptChoice::Cancel, tl!("Start in Safe Mode").to_string())
}

/// The choice's Disable and Continue (`normal` false) or Start Normally: Safe Mode ends and the
/// plug-ins load, without the stopped ones for Disable and Continue (kept off for later launches
/// too).
fn confirm_stopped(app: &mut VectorcraftApp, normal: bool) {
    let items = stopped_items(&app.ui.stopped_plugin.take().unwrap_or(Value::Null));
    // When a disabled plug-in can't be saved for the next launch, that launch asks again. Every
    // listed plug-in is turned off for this launch, whether or not the one before it was saved.
    let saved = normal || {
        let mut all = true;
        for item in items {
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
}

/// Help › Safe Mode: the disabled plug-ins, none ticked for Enable on Re-launch. A plug-in disabled
/// by its id and found under several files has an entry for each file; it is listed once, and
/// Enable on Re-launch turns on every entry with its id.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let disabled = safe_mode::disabled();
    let items: Vec<&Item> =
        disabled.iter().enumerate().filter(|(k, i)| i.id.is_none() || !disabled[..*k].iter().any(|e| e.id == i.id)).map(|(_, i)| i).collect();
    let mut fields = DialogFields::new();
    let mut params = Vec::new();
    if safe_mode::active() {
        params.push(
            Param::read_only_text("note", "", tl!("VectorCraft is in Safe Mode: no plug-ins load or run until it starts normally.").to_string())
                .full_width(),
        );
    }
    params.push(Param::section(tl!("Disabled plug-ins").to_string()));
    if items.is_empty() {
        params.push(Param::read_only_text("none", "", tl!("No plug-ins are disabled.").to_string()).full_width());
    }
    for (k, item) in items.iter().enumerate() {
        // A name made safe to show: control and reordering characters removed, bounded.
        let label = format!("{}: {}", safe_mode::display_name(&item.label()), tl!("Enable on Re-launch"));
        params.push(Param::new(enable_key(k), label, ParamKind::bool(false)));
        fields.set(item_key(k), FieldValue::Json(item.to_json()));
    }
    // PLAN: `open` has no context, and the dialog remembers no values, so none is read from it; assumed `VectorcraftApp::dialogs()` (wg.S1).
    let dialog = HostedParamDialog::new(&egui::Context::default(), KIND, fields, move |values| {
        ParamDialog::new(KIND, tl!("Safe Mode").to_string(), params.clone()).values(Bind::bound(values))
    });
    app.dialogs().ok_or("no window")?.with(|h| {
        h.open(dialog);
    });
    Ok(Value::Null)
}

/// Help › Safe Mode's OK: each ticked plug-in loads again from the next launch
/// (`plugin.setEnabled`).
fn confirm(app: &mut VectorcraftApp, fields: &DialogFields) {
    let mut errors = vec![];
    let mut unsaved = false;
    let mut k = 0;
    while let Some(FieldValue::Json(v)) = fields.get(&item_key(k)) {
        let ticked = fields.bool(&enable_key(k)) == Some(true);
        k += 1;
        if !ticked {
            continue;
        }
        let p = match (v["id"].as_str(), v["file"].as_str()) {
            (Some(id), _) => serde_json::json!({"id": id, "enabled": true}),
            (None, Some(file)) => serde_json::json!({"file": file, "enabled": true}),
            (None, None) => continue,
        };
        match app.run("plugin.setEnabled", p) {
            Ok(r) => unsaved |= r["saved"] == false,
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        app.status(errors.join("; "));
    } else if unsaved {
        not_saved(app);
    }
}

/// The disabled plug-ins couldn't be written to the plug-in folder: the next launch doesn't have
/// the change, which goes with the next change that saves.
fn not_saved(app: &mut VectorcraftApp) {
    app.status("Couldn't save the disabled plug-ins for the next launch");
}

/// After the shell pass: puts the choice about a stopped plug-in on the host when nothing else
/// holds it, and carries out the answers of the choice and of Help › Safe Mode.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    pump_stopped(app, ctx);
    pump_help(app);
}

fn pump_stopped(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(fields) = app.ui.stopped_plugin.clone() else { return };
    // PLAN: assumed `VectorcraftApp::dialogs() -> Option<handle>` whose `with(|host: &mut DialogHost| ..)` runs on the host (wg.S1).
    let Some(dialogs) = app.dialogs() else { return };
    let answered = dialogs.with(|h| {
        let answered = h.take::<StandardPrompt>();
        // An empty host has neither an open dialog nor a queued one: otherwise the choice waits. It opens now rather
        // than through the startup queue, which the host drains only in its next pass: a frame may have no other.
        if answered.is_none() && h.is_empty() {
            h.open(stopped_prompt(&fields));
            ctx.request_repaint();
        }
        answered
    });
    match answered.map(|r| r.ok().map(|a| a.choice)) {
        None => {}
        Some(Some(PromptChoice::Proceed)) => confirm_stopped(app, false),
        Some(Some(PromptChoice::Alternate)) => confirm_stopped(app, true),
        Some(_) => cancel_stopped(app),
    }
}

fn pump_help(app: &mut VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    // A dialog made with an instance keeps its result apart from every other form's.
    let answered = dialogs.with(|h| h.take_by_id::<HostedParamDialog>(dialog_id(KIND)));
    if let Some(DialogResult::Ok(result)) = answered {
        confirm(app, &result.fields);
    }
}
