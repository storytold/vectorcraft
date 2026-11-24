//! Edit → Transparency Flattener Presets: every preset in a list (built-in ones in brackets, read
//! only) with the selected one's name and options beside it, edited in place on saved presets
//! (`flattener.presets.save`); New (a copy of the selected preset), Delete, Import… and Export…
//! (the selected preset) under the list. Close keeps everything: each change is saved at once.
//!
//! Fields: `selected` (a preset's name), then the selected preset's `name` and option keys
//! (setting them on a saved preset saves it; a new `name` renames it; a built-in preset refuses
//! them).

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::flatten::{PRESET_EXTS, PRESET_FORMAT};
use vectorcraft_engine::cmd::{FlattenOptions, FlattenerPreset};

use super::preset_manager::{self, Detail, ManagerAction, ManagerEvent, ManagerRow, Source};
use super::{flatten, form};
use crate::VectorcraftApp;

/// The dialog key of Transparency Flattener Presets.
const KEY: &str = "flattenerPresets";

/// How the preset manager reads and changes flattener presets.
pub(super) static SOURCE: Source = Source {
    key: KEY,
    title: || tl!("Transparency Flattener Presets").to_string(),
    actions: &[ManagerAction::New, ManagerAction::Delete, ManagerAction::Import, ManagerAction::Export],
    editor: false,
    renames: true,
    rows,
    stored,
    detail,
    run,
};

/// Open the presets manager on `selected` (default: the first preset), showing its name and options.
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    let first = FlattenOptions::preset_label(FlattenOptions::PRESETS[0]).unwrap_or_default();
    preset_manager::open(app, &SOURCE, selected.unwrap_or(first));
}

/// The presets, the built-in ones first.
fn rows(app: &VectorcraftApp) -> Vec<ManagerRow> {
    let built_ins = FlattenOptions::PRESETS.len();
    app.session.flattener_presets().iter().enumerate().map(|(i, preset)| ManagerRow::new(preset.name.clone(), i < built_ins)).collect()
}

/// The stored preset `name`.
fn stored(app: &VectorcraftApp, name: &str) -> Value {
    let found = app.session.flattener_presets().into_iter().find(|preset| preset.name == name);
    found.and_then(|preset| serde_json::to_value(preset).ok()).unwrap_or_default()
}

/// The details of the preset `name`: its options, read only on a built-in preset.
fn detail(app: &VectorcraftApp, name: &str) -> Vec<Detail> {
    let presets = app.session.flattener_presets();
    let Some((at, preset)) = presets.iter().enumerate().find(|(_, preset)| preset.name == name) else { return Vec::new() };
    let built_in = at < FlattenOptions::PRESETS.len();
    let params = flatten::option_params(&[], "");
    let mut map = Map::new();
    flatten::put_options(&mut map, &preset.options);
    let mut out = Vec::new();
    // A saved preset's name is the manager's Name field.
    if built_in {
        out.push(Detail::Line(tl!("Name").to_string(), preset.name.clone()));
    }
    let fields = form::fields_of(&params, &map);
    out.push(Detail::Rows(params, fields, built_in));
    if built_in {
        out.push(Detail::Note(tl!("Built-in presets don't change: New makes a copy you can edit.").to_string()));
    }
    out
}

/// The saved or built-in preset `name`, with whether it is a built-in one.
fn find(app: &VectorcraftApp, name: &str) -> Option<(FlattenerPreset, bool)> {
    let presets = app.session.flattener_presets();
    let (at, preset) = presets.into_iter().enumerate().find(|(_, preset)| preset.name == name)?;
    Some((preset, at < FlattenOptions::PRESETS.len()))
}

/// Carries out `event` on the preset `current`.
fn run(app: &mut VectorcraftApp, event: &ManagerEvent, current: &str) -> Result<Option<String>, String> {
    let select = |saved: Value| saved["name"].as_str().map(str::to_owned);
    match event {
        ManagerEvent::Select(_) | ManagerEvent::Action(ManagerAction::Edit) => Ok(None),
        ManagerEvent::Action(ManagerAction::New) => Ok(select(app.run("flattener.presets.save", json!({ "preset": current }))?)),
        ManagerEvent::Action(ManagerAction::Delete) => {
            let i = app.session.prefs.flattener_presets.iter().position(|p| p.name == current);
            app.run("flattener.presets.delete", json!({ "name": current }))?;
            let above = preset_manager::above(i, FlattenOptions::PRESETS.len());
            Ok(app.session.flattener_presets().get(above).map(|p| p.name.clone()))
        }
        ManagerEvent::Action(ManagerAction::Import) => preset_manager::import(app, KEY, "flattener.presets.import", "Flattener presets", PRESET_EXTS),
        ManagerEvent::Action(ManagerAction::Export) => {
            crate::io::save_command_output(app, "flattener.presets.export", PRESET_FORMAT, json!({ "names": [current] }), |_, _| Ok(Value::Null))?;
            Ok(None)
        }
        // A new name that is taken or a built-in one is refused by the command.
        ManagerEvent::Rename(name) if name.trim() == current => Ok(None),
        ManagerEvent::Rename(name) => Ok(select(app.run("flattener.presets.save", json!({ "name": current, "newName": name }))?)),
        ManagerEvent::Edited(fields) => {
            let Some((preset, false)) = find(app, current) else { return Ok(None) };
            let params = flatten::option_params(&[], "");
            let options = FlattenOptions::from_params(&Value::Object(form::json_of(&params, fields))).map_err(|e| e.to_string())?;
            if options != preset.options {
                app.run("flattener.presets.save", json!({ "name": current, "options": options }))?;
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::{StackDialog as _, dialog_id};
    use vectorcraft_engine::Session;

    use super::super::preset_manager::PresetManager;

    /// An app with a window, whose dialog host the manager opens on.
    fn app() -> VectorcraftApp {
        crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()))
    }

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().expect("a window")
    }

    fn host(app: &VectorcraftApp) -> craft::ui::shell::WindowDialogs {
        app.dialogs().expect("a window")
    }

    /// One app frame: the host's pump carries out what the manager queued, and answered panels are taken.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = ctx(app);
        crate::testing::app_frame(app, &ctx, egui::RawInput::default());
    }

    /// Press a manager button as the user does: the manager queues it and the pump carries it out.
    fn press(app: &mut VectorcraftApp, action: ManagerAction) {
        host(app).with(|h| h.get_mut_by_id::<PresetManager>(dialog_id(KEY)).expect("manager is open").queue(ManagerEvent::Action(action)));
        frame(app);
    }

    fn fields(app: &mut VectorcraftApp) -> Value {
        host(app).with(|h| h.get_by_id::<PresetManager>(dialog_id(KEY)).and_then(|m| m.json_fields())).expect("manager is open")
    }

    /// `ui.dialog.set` on the open manager, then a frame.
    fn set(app: &mut VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let ctx = ctx(app);
        let set = host(app).with(|h| h.set_json_field(&ctx, dialog_id(KEY), field, value));
        frame(app);
        set
    }

    #[test]
    fn edits_save_saved_presets_and_leave_built_in_ones() {
        let mut app = app();
        open(&mut app, None);
        let shown = fields(&mut app);
        assert_eq!((shown["selected"].clone(), shown["lineArtPpi"].as_f64()), (json!("High Resolution"), Some(1200.0)));
        // Built-in presets refuse edits and keep their values.
        assert!(set(&mut app, "balance", json!(10)).is_err());
        assert_eq!(fields(&mut app)["balance"].as_f64(), Some(100.0));
        assert!(app.session.prefs.flattener_presets.is_empty());
        // New makes an editable copy of the selected preset and selects it.
        press(&mut app, ManagerAction::New);
        assert_eq!(fields(&mut app)["selected"], "Flattener Preset 1");
        assert_eq!(app.session.prefs.flattener_presets[0].options, FlattenOptions::preset("high").unwrap());
        // Its options and name save as they change.
        set(&mut app, "balance", json!(10)).unwrap();
        assert_eq!(app.session.prefs.flattener_presets[0].options.balance, 10.0);
        set(&mut app, "name", json!("Coarse")).unwrap();
        assert_eq!(app.session.prefs.flattener_presets[0].name, "Coarse");
        let shown = fields(&mut app);
        assert_eq!((shown["selected"].clone(), shown["balance"].as_f64()), (json!("Coarse"), Some(10.0)));
        // A taken name is refused and the name goes back.
        set(&mut app, "name", json!("Low Resolution")).unwrap();
        assert_eq!(fields(&mut app)["name"], "Coarse");
        // Delete selects the preset above it; Close keeps what was saved.
        press(&mut app, ManagerAction::Delete);
        assert_eq!(fields(&mut app)["selected"], "Low Resolution");
        assert!(app.session.prefs.flattener_presets.is_empty());
        let (id, ctx) = (dialog_id(KEY), ctx(&app));
        host(&app).with(|h| h.confirm_by_id(&ctx, id));
        assert!(host(&app).with(|h| h.get_by_id::<PresetManager>(id).is_none()), "Close");
    }

    #[test]
    fn saved_presets_persist_with_the_ui_state_and_preferences_still_apply() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("flattener.presets.save", json!({"name": "Kept", "lineArtPpi": 600})).unwrap();
        // Preferences OK with saved presets around (they aren't preference fields).
        app.run("edit.preferences", json!({})).unwrap();
        crate::prefs_dialog::confirm(&mut app).unwrap();
        crate::prefs_dialog::snapshot(&mut app);
        let saved = serde_json::to_vec(&app.ui).unwrap();
        let mut back = VectorcraftApp::new(Session::new(), Default::default());
        back.ui = serde_json::from_slice::<crate::UiState>(&saved).unwrap().sanitized();
        crate::prefs_dialog::restore(&mut back);
        assert_eq!(back.session.prefs.flattener_presets, app.session.prefs.flattener_presets);
        assert_eq!(back.session.flatten_options(&json!({"preset": "Kept"})).unwrap().line_art_ppi, 600.0);
    }

    #[test]
    fn import_and_export_go_through_files() {
        let dir = std::env::temp_dir().join(format!("vc-flattener-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mine.vcflattener").to_string_lossy().to_string();
        let services =
            crate::Services { write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))), ..Default::default() };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20})).unwrap();
        // The scripted file service gives the app its window too.
        let picker = crate::tests::picks::scripted(&mut app);
        open(&mut app, Some("Mine"));
        press(&mut app, ManagerAction::Export);
        // The answered save panel writes the file in the next frame.
        picker.answer_saved(&file, None);
        frame(&mut app);
        // Opening the file imports it (also how the web's picked file arrives).
        let bytes = std::fs::read(&file).unwrap();
        crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
        let names: Vec<&str> = app.session.prefs.flattener_presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Mine", "Mine 2"]);
        assert_eq!(app.session.prefs.flattener_presets[1].options.balance, 20.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
