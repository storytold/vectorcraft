//! Edit → Print Presets: every preset in a list ([Default], protected, then the saved ones) with
//! how the selected one differs from [Default] beside it; New… (a preset starting from the
//! selected one), Edit… (saved presets), Delete, Import… and Export… (the selected preset) under
//! the list. New… and Edit… open the preset editor (the Print dialog in its editor mode), whose OK
//! comes back here. The manager is the shared presets manager (`preset_manager`); this module is
//! what is particular to print presets.

// PLAN: assumed `preset_manager` API (export-w19): `PresetManager::new(key, title, rows, actions).selected(name)`,
// `ManagerRow::new(key, label).protected()`, `Detail::{Title, Heading, Line, Note}`, `ManagerAction::{New, Edit, Delete,
// Import, Export}`, `ManagerEvent::{Select(name), Action(ManagerAction)}`.
use craft::host::files::PickRequest;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::printpresets::{DEFAULT_PRESET, PRESET_EXTS, PRESET_FORMAT, changes};

use super::preset_manager::{Detail, ManagerAction, ManagerEvent, ManagerRow, PresetManager};
use super::print::{changed_lines, open_preset, section_of};
use crate::VectorcraftApp;
use crate::file_asks::{Owner, ask};

/// The key of the Print Presets dialog.
pub(super) const KEY: &str = "printPresets";

/// Open the presets manager on `selected` (default: [Default]).
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    let Some(dialogs) = app.dialogs() else { return };
    let actions = vec![ManagerAction::New, ManagerAction::Edit, ManagerAction::Delete, ManagerAction::Import, ManagerAction::Export];
    let manager = PresetManager::new(KEY, tl!("Print Presets"), rows(app), actions).selected(selected.unwrap_or(DEFAULT_PRESET));
    dialogs.with(|h| h.open(manager));
}

/// The list: [Default] (protected, translated), then the saved presets.
pub(super) fn rows(app: &VectorcraftApp) -> Vec<ManagerRow> {
    std::iter::once(ManagerRow::new(DEFAULT_PRESET, tl!(DEFAULT_PRESET)).protected())
        .chain(app.session.prefs.print_presets.iter().map(|p| ManagerRow::new(p.name.as_str(), p.name.as_str())))
        .collect()
}

/// What the manager shows beside the list for the preset `name`: how it differs from [Default].
pub(super) fn detail(app: &VectorcraftApp, name: &str) -> Vec<Detail> {
    let builtin = name == DEFAULT_PRESET;
    let title = if builtin { tl!(DEFAULT_PRESET).to_string() } else { name.to_string() };
    let mut out = vec![Detail::Title(title), Detail::Heading(crate::i18n::fmt(tl!("Differences from {preset}:"), &[("preset", DEFAULT_PRESET)]))];
    let settings = app.session.prefs.print_presets.iter().find(|p| p.name == name).map(|p| &p.settings);
    let mut changed = settings.map(|s| changes(s, &Default::default())).unwrap_or_default();
    changed.sort_by_key(|c| section_of(c["option"].as_str().unwrap_or_default()).0);
    let lines = changed_lines(&changed);
    if lines.is_empty() {
        out.push(Detail::Note(tl!("None.").to_string()));
    }
    out.extend(lines.into_iter().map(Detail::Line));
    if builtin {
        out.push(Detail::Note(tl!("[Default] is protected: New… starts an editable copy.").to_string()));
    }
    out
}

/// Do what a manager event asks of the preset `current`; the preset to select afterwards, when it changes.
pub(super) fn run(app: &mut VectorcraftApp, event: &ManagerEvent, current: &str) -> Result<Option<String>, String> {
    let action = match event {
        ManagerEvent::Select(name) => return Ok(Some(name.clone())),
        ManagerEvent::Action(action) => action,
    };
    match action {
        // The editor replaces the manager; its OK comes back here.
        ManagerAction::New | ManagerAction::Edit => {
            let params = if matches!(action, ManagerAction::New) { json!({ "preset": current }) } else { json!({ "name": current }) };
            open_preset(app, &params)?;
            // PLAN: assumed `app.ui_ctx: Option<egui::Context>` (lib.rs:203).
            if let (Some(dialogs), Some(ctx)) = (app.dialogs(), app.ui_ctx.clone()) {
                dialogs.with(|h| h.cancel::<PresetManager>(&ctx));
            }
            Ok(None)
        }
        ManagerAction::Delete => {
            let i = app.session.prefs.print_presets.iter().position(|p| p.name == current);
            app.run("print.presets.delete", json!({ "name": current }))?;
            // The one above it takes its place ([Default] above the first saved one).
            let next = i.and_then(|i| i.checked_sub(1)).and_then(|i| app.session.prefs.print_presets.get(i)).map(|p| p.name.clone());
            Ok(Some(next.unwrap_or_else(|| DEFAULT_PRESET.to_string())))
        }
        ManagerAction::Import => {
            let request = PickRequest::open("print-presets").filter("Print presets", PRESET_EXTS);
            // PLAN: assumed `Picked::path(&self) -> Option<String>`.
            ask(app, request, Owner::Free, |app, picked| {
                let path = picked.path().ok_or("cancelled")?;
                let r = app.run("print.presets.import", json!({ "path": path }))?;
                open(app, r["imported"].get(0).and_then(Value::as_str));
                Ok(r)
            });
            Ok(None)
        }
        ManagerAction::Export => {
            crate::io::save_command_output(app, "print.presets.export", PRESET_FORMAT, json!({ "names": [current] }))?;
            Ok(None)
        }
    }
}
