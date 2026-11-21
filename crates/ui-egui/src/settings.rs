//! The preferences file: `config.toml` in the settings folder, held by craft-settings' store.
//! The UI state is the one saved section; the engine preferences ride in `UiState::engine_prefs`.

use craft::settings::{SectionError, Settings};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{UiState, VectorcraftApp};

/// What VectorCraft saves between runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// The UI state with the engine preferences (`engine_prefs`).
    pub ui: UiState,
}

impl Settings for AppSettings {
    const APP_ID: &'static str = "vectorcraft";
    const FOLDER_NAME: &'static str = "VectorCraft";
    const SCHEMA_VERSION: u32 = 1;
}

/// Reads an old `ui.json` (VectorCraft's or DrawCraft's): older layouts converted, what this version
/// cannot read of the docking data dropped and reported as a reset `docking` value.
pub fn parse_legacy(bytes: &[u8]) -> Result<(AppSettings, Vec<SectionError>), String> {
    let mut raw: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    name_current_action_set(&mut raw);
    let bytes = serde_json::to_vec(&raw).map_err(|e| e.to_string())?;
    let read = UiState::read_preferences(&bytes).ok_or_else(|| "the preferences are not readable".to_string())?;
    let errors = if read.intact {
        Vec::new()
    } else {
        vec![SectionError { section: "docking".into(), message: "part of the saved layout could not be read and was reset".into() }]
    };
    Ok((AppSettings { ui: read.ui }, errors))
}

/// Old files saved the current Actions set as an index; sets are identified by name now, so another
/// window's added or removed sets do not move the choice.
fn name_current_action_set(raw: &mut Value) {
    let Some(map) = raw.as_object_mut() else { return };
    let Some(index) = map.get("action_set").and_then(Value::as_u64) else { return };
    let name = usize::try_from(index).ok().and_then(|i| map.get("action_sets")?.get(i)?.get("name")).cloned();
    match name {
        Some(name) => map.insert("action_set".into(), name),
        None => map.remove("action_set"),
    };
}

/// The settings `app` would save now. What a start resets (the status line, the popped-out panel,
/// the palette) is left at its default, so it never makes a change to save.
pub fn from_app(app: &VectorcraftApp) -> AppSettings {
    let mut ui = app.ui.clone();
    ui.palette_open = false;
    ui.palette_query.clear();
    ui.status.clear();
    ui.open_panel = None;
    // TOML has no null: the engine preferences are always a table.
    let prefs = app.session.prefs.to_json();
    ui.engine_prefs = if prefs.is_null() { Value::Object(Default::default()) } else { prefs };
    AppSettings { ui }
}
