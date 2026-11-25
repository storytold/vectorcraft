//! Edit → Print Presets: every preset in a list ([Default], protected, then the saved ones) with
//! how the selected one differs from [Default] beside it; New… (a preset starting from the
//! selected one), Edit… (saved presets), Delete, Import… and Export… (the selected preset) under
//! the list. New… and Edit… open the preset editor (the Print dialog in its editor mode), whose OK
//! comes back here. The manager is the shared presets manager (`preset_manager`); this module is
//! what is particular to print presets.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::printpresets::{DEFAULT_PRESET, PRESET_EXTS, PRESET_FORMAT, changes};

use super::preset_manager::{self, Detail, ManagerAction, ManagerEvent, ManagerRow, Source};
use super::print::{changed_lines, open_preset, section_of};
use crate::VectorcraftApp;

/// The key of the Print Presets dialog.
const KEY: &str = "printPresets";

/// How the preset manager reads and changes print presets.
pub(super) static SOURCE: Source = Source {
    key: KEY,
    title: || tl!("Print Presets").to_string(),
    actions: &[ManagerAction::New, ManagerAction::Edit, ManagerAction::Delete, ManagerAction::Import, ManagerAction::Export],
    editor: true,
    renames: false,
    rows,
    stored,
    detail,
    run,
};

/// Open the presets manager on `selected` (default: [Default]).
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    preset_manager::open(app, &SOURCE, selected.unwrap_or(DEFAULT_PRESET));
}

/// The list: [Default] (protected, translated; its name carries its brackets), then the saved presets.
fn rows(app: &VectorcraftApp) -> Vec<ManagerRow> {
    let default = ManagerRow { name: DEFAULT_PRESET.to_string(), label: tl!(DEFAULT_PRESET).to_string(), built_in: true };
    std::iter::once(default).chain(app.session.prefs.print_presets.iter().map(|p| ManagerRow::new(p.name.as_str(), false))).collect()
}

/// The stored settings of the preset `name`.
fn stored(app: &VectorcraftApp, name: &str) -> Value {
    app.session.print_preset_settings(name).and_then(|set| serde_json::to_value(set).ok()).unwrap_or_default()
}

/// What the manager shows beside the list for the preset `name`: how it differs from [Default].
fn detail(app: &VectorcraftApp, name: &str) -> Vec<Detail> {
    let builtin = name == DEFAULT_PRESET;
    let title = if builtin { tl!(DEFAULT_PRESET).to_string() } else { name.to_string() };
    let mut out = vec![Detail::Title(title), Detail::Note(crate::i18n::fmt(tl!("Differences from {preset}:"), &[("preset", DEFAULT_PRESET)]))];
    let settings = app.session.prefs.print_presets.iter().find(|p| p.name == name).map(|p| &p.settings);
    let mut changed = settings.map(|s| changes(s, &Default::default())).unwrap_or_default();
    changed.sort_by_key(|c| section_of(c["option"].as_str().unwrap_or_default()).0);
    let lines = changed_lines(&changed);
    if lines.is_empty() {
        out.push(Detail::Note(tl!("None.").to_string()));
    }
    out.extend(lines.into_iter().map(Detail::Text));
    if builtin {
        out.push(Detail::Note(tl!("[Default] is protected: New… starts an editable copy.").to_string()));
    }
    out
}

/// Carries out `event` on the preset `current`; the preset to select afterwards, when it changes.
fn run(app: &mut VectorcraftApp, event: &ManagerEvent, current: &str) -> Result<Option<String>, String> {
    let ManagerEvent::Action(action) = event else { return Ok(None) };
    match action {
        // The editor replaces the manager (the pump closes it); the editor's OK comes back here.
        ManagerAction::New => open_preset(app, &json!({ "preset": current })).map(|_| None),
        ManagerAction::Edit => open_preset(app, &json!({ "name": current })).map(|_| None),
        ManagerAction::Delete => {
            let i = app.session.prefs.print_presets.iter().position(|p| p.name == current);
            app.run("print.presets.delete", json!({ "name": current }))?;
            // The one above it takes its place ([Default] above the first saved one).
            Ok(rows(app).get(preset_manager::above(i, 1)).map(|row| row.name.clone()))
        }
        ManagerAction::Import => preset_manager::import(app, KEY, "print.presets.import", "Print presets", PRESET_EXTS),
        ManagerAction::Export => {
            crate::io::save_command_output(app, "print.presets.export", PRESET_FORMAT, json!({ "names": [current] }), |_, _| Ok(Value::Null))?;
            Ok(None)
        }
    }
}
