//! Edit → Transparency Flattener Presets: every preset in a list (built-in ones in brackets, read
//! only) with the selected one's name and options beside it, edited in place on saved presets
//! (`flattener.presets.save`); New (a copy of the selected preset), Delete, Import… and Export…
//! (the selected preset) under the list. Close keeps everything: each change is saved at once.
//!
//! Fields: `selected` (a preset's name), then the selected preset's `name` and option keys
//! (setting them on a saved preset saves it; a new `name` renames it once the field isn't being
//! edited; on a built-in preset they go back).

use serde_json::{Value, json};
use vectorcraft_engine::cmd::FlattenOptions;
use vectorcraft_engine::cmd::flatten::{PRESET_EXTS, PRESET_FORMAT};

use super::flatten::{options_editor, put_options};
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Transparency Flattener Presets.
pub const KIND: &str = "flattenerPresets";

/// The preset whose name and options the fields hold.
const SHOWN: &str = "__shown";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Transparency Flattener Presets").into(),
    body,
    confirm: |app, _| {
        app.ui.dialog = None;
        Ok(Value::Null)
    },
    ok: None,
    min_width: 620.0,
    ..DialogSpec::FORM
};

/// Open the presets manager on `selected` (default: the first preset).
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    let first = FlattenOptions::preset_label(FlattenOptions::PRESETS[0]).unwrap_or_default();
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "selected": selected.unwrap_or(first) })));
}

/// What a button asked for, done once the list and fields are drawn.
enum Action {
    Select(String),
    New,
    Delete,
    Import,
    Export,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let presets = app.session.flattener_presets();
    let builtins = FlattenOptions::PRESETS.len();
    let i = presets.iter().position(|p| p.name.eq_ignore_ascii_case(d.str("selected").trim())).unwrap_or(0);
    let (preset, builtin) = (&presets[i], i < builtins);
    if d.str(SHOWN) != preset.name {
        put_options(&mut d.fields, &preset.options);
        for k in ["selected", "name", SHOWN] {
            d.fields.insert(k.into(), json!(preset.name));
        }
    }
    let mut act = None;
    let mut renaming = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(232.0);
            widgets::dim_label(ui, tl!("Presets:"));
            widgets::list_box(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("flattener-presets").min_scrolled_height(250.0).max_height(250.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(250.0);
                    for (k, p) in presets.iter().enumerate() {
                        let label = if k < builtins { format!("[{}]", tl!(&p.name)) } else { p.name.clone() };
                        if ui.selectable_label(k == i, label).clicked() {
                            act = Some(Action::Select(p.name.clone()));
                        }
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if widgets::flat_button(ui, tl!("New"), 44.0).on_hover_text(tl!("A saved copy of the selected preset")).clicked() {
                    act = Some(Action::New);
                }
                if ui.add_enabled_ui(!builtin, |ui| widgets::flat_button(ui, tl!("Delete"), 52.0)).inner.clicked() {
                    act = Some(Action::Delete);
                }
                if widgets::flat_button(ui, tl!("Import…"), 62.0).clicked() {
                    act = Some(Action::Import);
                }
                if widgets::flat_button(ui, tl!("Export…"), 62.0).on_hover_text(tl!("Save the selected preset to a file")).clicked() {
                    act = Some(Action::Export);
                }
            });
        });
        ui.add_space(18.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                widgets::dim_label(ui, tl!("Name:"));
                if builtin {
                    ui.label(egui::RichText::new(&preset.name).color(t.text_strong));
                } else {
                    renaming = form::text_edit(ui, d, "name", 220.0).has_focus();
                }
            });
            ui.add_space(8.0);
            let mut o = FlattenOptions::from_params(&Value::Object(d.fields.clone())).unwrap_or_else(|_| preset.options.clone());
            if options_editor(ui, "flattener-presets", &mut o, !builtin) {
                put_options(&mut d.fields, &o);
            }
            if builtin {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl!("Built-in presets don't change: New makes a copy you can edit.")).color(t.text_dim).size(11.5));
            }
        });
    });
    // The fields (whether a widget or `ui.dialog.set` changed them) against the stored preset.
    let fields = FlattenOptions::from_params(&Value::Object(d.fields.clone()));
    if builtin {
        if fields.as_ref() != Ok(&preset.options) {
            put_options(&mut d.fields, &preset.options);
        }
    } else {
        match fields {
            Ok(o) if o != preset.options => {
                if let Err(e) = app.run("flattener.presets.save", json!({"name": preset.name, "options": o})) {
                    app.status(e);
                }
            }
            Ok(_) => {}
            Err(e) => app.status(e),
        }
        let name = d.str("name");
        if !renaming && name.trim() != preset.name {
            match app.run("flattener.presets.save", json!({"name": preset.name, "newName": name})) {
                Ok(r) => select(d, r["name"].as_str().unwrap_or_default()),
                Err(e) => {
                    app.status(e);
                    d.fields.insert("name".into(), json!(preset.name));
                }
            }
        }
    }
    if let Some(a) = act {
        let r = match a {
            // Their file dialogs, shown off the UI thread, import into or export from the dialog
            // as it is then.
            Action::Import | Action::Export => {
                let (current, export) = (preset.name.clone(), matches!(a, Action::Export));
                crate::picks::in_dialog(app, d, move |app, d| run(app, d, if export { Action::Export } else { Action::Import }, &current))
            }
            a => run(app, d, a, &preset.name),
        };
        if let Err(e) = r {
            app.status(e);
        }
    }
    false
}

/// Select the preset `name` (its values load on the next frame).
fn select(d: &mut Dialog, name: &str) {
    d.fields.insert("selected".into(), json!(name));
    d.fields.remove(SHOWN);
}

fn run(app: &mut VectorcraftApp, d: &mut Dialog, act: Action, current: &str) -> Result<(), String> {
    match act {
        Action::Select(name) => select(d, &name),
        Action::New => {
            let r = app.run("flattener.presets.save", json!({ "preset": current }))?;
            select(d, r["name"].as_str().unwrap_or_default());
        }
        Action::Delete => {
            let i = app.session.prefs.flattener_presets.iter().position(|p| p.name == current);
            app.run("flattener.presets.delete", json!({ "name": current }))?;
            // The one above it takes its place (the last built-in one above the first saved one).
            let next =
                app.session.flattener_presets().get((i.unwrap_or(0) + FlattenOptions::PRESETS.len()).saturating_sub(1)).map(|p| p.name.clone());
            select(d, &next.unwrap_or_default());
        }
        Action::Import => {
            // On the web the picked file arrives later and imports through `io::open_bytes`.
            if let Some(f) = app.services.open_async.as_mut() {
                f();
                return Ok(());
            }
            let pick = crate::FilePick { filters: vec![("Flattener presets", PRESET_EXTS)], ..Default::default() };
            let path = crate::picks::open(app, &pick).ok_or("cancelled")?;
            let r = app.run("flattener.presets.import", json!({ "path": path }))?;
            if let Some(first) = r["imported"].get(0).and_then(Value::as_str) {
                select(d, first);
            }
        }
        Action::Export => {
            crate::io::save_command_output(app, "flattener.presets.export", PRESET_FORMAT, json!({ "names": [current] }))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
        frame(app);
    }

    fn field(app: &VectorcraftApp, k: &str) -> Value {
        app.ui.dialog.as_ref().unwrap().fields.get(k).cloned().unwrap_or_default()
    }

    #[test]
    fn edits_save_saved_presets_and_leave_built_in_ones() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("ui.flattenerPresetsDialog", json!({})).unwrap();
        frame(&mut app);
        assert_eq!((field(&app, "selected"), field(&app, "lineArtPpi")), (json!("High Resolution"), json!(1200.0)));
        // Built-in presets go back to their values.
        set(&mut app, "balance", json!(10));
        assert_eq!(field(&app, "balance"), json!(100.0));
        frame(&mut app);
        assert!(app.session.prefs.flattener_presets.is_empty());
        // New makes an editable copy of the selected preset and selects it.
        let mut d = app.ui.dialog.clone().unwrap();
        run(&mut app, &mut d, Action::New, "High Resolution").unwrap();
        app.ui.dialog = Some(d);
        frame(&mut app);
        assert_eq!(field(&app, "selected"), json!("Flattener Preset 1"));
        assert_eq!(app.session.prefs.flattener_presets[0].options, FlattenOptions::preset("high").unwrap());
        // Its options and name save as they change.
        set(&mut app, "balance", json!(10));
        assert_eq!(app.session.prefs.flattener_presets[0].options.balance, 10.0);
        set(&mut app, "name", json!("Coarse"));
        assert_eq!(app.session.prefs.flattener_presets[0].name, "Coarse");
        frame(&mut app);
        assert_eq!((field(&app, "selected"), field(&app, "balance")), (json!("Coarse"), json!(10.0)));
        // A taken name is refused and the field goes back.
        set(&mut app, "name", json!("Low Resolution"));
        assert_eq!(field(&app, "name"), json!("Coarse"));
        // Delete selects the preset above it; Close keeps what was saved.
        let mut d = app.ui.dialog.clone().unwrap();
        run(&mut app, &mut d, Action::Delete, "Coarse").unwrap();
        assert_eq!(d.str("selected"), "Low Resolution");
        assert!(app.session.prefs.flattener_presets.is_empty());
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
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
        let picked = file.clone();
        let services = crate::Services {
            pick_save: Some(Box::new(move |_: &crate::FilePick| Some(picked.clone()))),
            write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20})).unwrap();
        app.run("ui.flattenerPresetsDialog", json!({"selected": "Mine"})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        run(&mut app, &mut d, Action::Export, "Mine").unwrap();
        // Opening the file imports it (also how the web's picked file arrives).
        let bytes = std::fs::read(&file).unwrap();
        crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
        let names: Vec<&str> = app.session.prefs.flattener_presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Mine", "Mine 2"]);
        assert_eq!(app.session.prefs.flattener_presets[1].options.balance, 20.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
