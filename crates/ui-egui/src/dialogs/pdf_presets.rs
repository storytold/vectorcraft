//! Edit → PDF Presets: every preset in a list (built-in ones in brackets, read only) with the
//! selected one's description and how its settings differ from the app default beside it; New…
//! (a preset starting from the selected one), Edit… (saved presets), Delete, Import… and Export…
//! (the selected preset) under the list. New… and Edit… open the preset editor (the Save PDF
//! dialog in its preset mode), whose OK comes back here.
//!
//! Fields: `selected` (a preset's name), then its `name`.

use std::sync::Arc;

use craft::ui::patterns::form_dialogs::{PdfExportWords, differences};
use craft::ui::runtime::env::Env;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::pdf;
use vectorcraft_engine::cmd::pdfcmds::{PRESET_EXTS, PRESET_FORMAT};

use super::preset_manager::{self, Detail, ManagerAction, ManagerEvent, ManagerRow, Source};
use super::save_pdf::open_preset;
use crate::VectorcraftApp;

/// The dialog key of PDF Presets.
const KEY: &str = "pdfPresets";

/// How the preset manager reads and changes PDF presets.
pub(super) static SOURCE: Source = Source {
    key: KEY,
    title: || tl!("PDF Presets").to_string(),
    actions: &[ManagerAction::New, ManagerAction::Edit, ManagerAction::Delete, ManagerAction::Import, ManagerAction::Export],
    editor: true,
    renames: false,
    rows,
    stored,
    detail,
    run,
};

/// Open the presets manager on `selected` (default: the app default).
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    preset_manager::open(app, &SOURCE, selected.unwrap_or(pdf::DEFAULT_PRESET));
}

/// The presets, the built-in ones first.
fn rows(app: &VectorcraftApp) -> Vec<ManagerRow> {
    let presets = app.session.pdf_presets();
    let built_ins = presets.len().saturating_sub(app.session.prefs.pdf_presets.len());
    presets.iter().enumerate().map(|(i, preset)| ManagerRow::new(preset.name.clone(), i < built_ins)).collect()
}

/// The stored preset `name`.
fn stored(app: &VectorcraftApp, name: &str) -> Value {
    let found = app.session.pdf_presets().into_iter().find(|preset| preset.name == name);
    found.and_then(|preset| serde_json::to_value(preset).ok()).unwrap_or_default()
}

/// The details of the preset `name`: its description, how its settings differ from the app default, and the
/// notes on what Save PDF can't honour or apply yet.
fn detail(app: &VectorcraftApp, name: &str) -> Vec<Detail> {
    let presets = app.session.pdf_presets();
    let built_ins = presets.len().saturating_sub(app.session.prefs.pdf_presets.len());
    let Some((at, preset)) = presets.iter().enumerate().find(|(_, preset)| preset.name == name) else { return Vec::new() };
    let mut out = vec![Detail::Title(preset.name.clone())];
    if !preset.description.is_empty() {
        out.push(Detail::Text(preset.description.clone()));
    }
    let settings = preset.settings.clone();
    out.push(Detail::Lazy(Arc::new(move |env: &Env| differences_of(env, &settings))));
    if at < built_ins {
        out.push(Detail::Note(tl!("Built-in presets are read-only: New… starts an editable copy.").to_string()));
    }
    out
}

/// What sets the preset apart: its settings that differ from the app default, worded with the framework's
/// words, then what is wrong with them.
fn differences_of(env: &Env, settings: &vectorcraft_pdf::PdfSettings) -> Vec<Detail> {
    let words = env.words::<PdfExportWords>();
    let base = vectorcraft_pdf::builtin_preset(pdf::DEFAULT_PRESET).map(|p| p.settings).unwrap_or_default();
    let mut out = vec![Detail::Note(crate::i18n::fmt(tl!("Differences from {preset}:"), &[("preset", pdf::DEFAULT_PRESET)]).to_string())];
    let lines = differences(&words, settings, &base);
    if lines.is_empty() {
        out.push(Detail::Note(tl!("None.").to_string()));
    }
    out.extend(lines.into_iter().map(Detail::Text));
    // PLAN: assumed `vectorcraft_pdf::ActiveProfiles` (ex.P4), the catalog `check` and `warnings` take.
    let catalog = vectorcraft_pdf::ActiveProfiles::default();
    if let Err(e) = settings.check(&catalog) {
        out.push(Detail::Warning(words.error_text(&e)));
    }
    out.extend(settings.warnings(&catalog).iter().map(|w| Detail::Warning(words.warning_text(w))));
    out
}

/// Carries out `event` on the preset `current`.
fn run(app: &mut VectorcraftApp, event: &ManagerEvent, current: &str) -> Result<Option<String>, String> {
    let ManagerEvent::Action(action) = event else { return Ok(None) };
    match action {
        // The editor replaces the manager (the pump closes it); the editor's OK comes back here.
        ManagerAction::New => open_preset(app, &json!({ "preset": current })).map(|_| None),
        ManagerAction::Edit => open_preset(app, &json!({ "name": current })).map(|_| None),
        ManagerAction::Delete => {
            let i = app.session.prefs.pdf_presets.iter().position(|p| p.name == current);
            app.run("pdf.preset.delete", json!({ "name": current }))?;
            let above = preset_manager::above(i, vectorcraft_pdf::builtin_presets().len());
            Ok(app.session.pdf_presets().get(above).map(|p| p.name.clone()))
        }
        ManagerAction::Import => preset_manager::import(app, KEY, "pdf.preset.import", "PDF presets", PRESET_EXTS),
        ManagerAction::Export => {
            crate::io::save_command_output(app, "pdf.preset.export", PRESET_FORMAT, json!({ "names": [current] }))?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::{StackDialog as _, dialog_id};
    use craft::ui::patterns::form_dialogs::HostedPdfExport;
    use vectorcraft_engine::Session;

    use super::super::preset_manager::PresetManager;

    // PLAN: assumed `VectorcraftApp::new` gives an app whose `dialogs()` host works without a window (wg.S1).
    fn app() -> VectorcraftApp {
        VectorcraftApp::new(Session::new(), Default::default())
    }

    /// One frame: the host's pump carries out what the manager queued.
    fn frame(app: &mut VectorcraftApp) {
        // PLAN: assumed `dialogs::host::pump(app, ctx)` (hub-w2).
        super::super::host::pump(app, &egui::Context::default());
    }

    /// Press a manager button as the user does: the manager queues it and the pump carries it out.
    fn press(app: &mut VectorcraftApp, action: ManagerAction) {
        let manager = app.dialogs().get_mut_by_id::<PresetManager>(dialog_id(KEY));
        manager.expect("manager is open").queue(ManagerEvent::Action(action));
        frame(app);
    }

    fn manager_fields(app: &mut VectorcraftApp) -> Value {
        app.dialogs().get_by_id::<PresetManager>(dialog_id(KEY)).and_then(|m| m.json_fields()).expect("manager is open")
    }

    fn editor_fields(app: &mut VectorcraftApp) -> Value {
        app.dialogs().get::<HostedPdfExport>().and_then(|e| e.json_fields()).expect("editor is open")
    }

    fn set_editor(app: &mut VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let id = app.dialogs().get::<HostedPdfExport>().map(|e| e.id()).expect("editor is open");
        app.dialogs().set_json_field(&egui::Context::default(), id, field, value)
    }

    /// OK in the editor, then the pump that saves it and reopens the manager.
    fn confirm_editor(app: &mut VectorcraftApp) -> bool {
        let ctx = egui::Context::default();
        let confirmed = app.dialogs().confirm::<HostedPdfExport>(&ctx);
        // PLAN: assumed `save_pdf::pump(app, ctx)` (export-w17).
        super::super::save_pdf::pump(app, &ctx);
        confirmed
    }

    #[test]
    fn new_edit_and_delete_go_through_the_preset_editor() {
        let mut app = app();
        open(&mut app, Some("Press Quality"));
        assert_eq!(manager_fields(&mut app)["selected"], "Press Quality");
        // New… opens the editor on a copy of the selected preset; its OK saves it and comes back.
        press(&mut app, ManagerAction::New);
        assert!(app.dialogs().get_by_id::<PresetManager>(dialog_id(KEY)).is_none(), "the manager makes room for the editor");
        let fields = editor_fields(&mut app);
        assert_eq!((fields["name"].clone(), fields["compatibility"].clone()), (json!("PDF Preset 1"), json!("1.6")));
        set_editor(&mut app, "name", json!("Press 1.5")).unwrap();
        set_editor(&mut app, "compatibility", json!("1.5")).unwrap();
        set_editor(&mut app, "description", json!("Mine")).unwrap();
        assert!(confirm_editor(&mut app));
        assert_eq!(manager_fields(&mut app)["selected"], "Press 1.5");
        let saved = &app.session.prefs.pdf_presets[0];
        assert_eq!((saved.settings.compatibility.id(), saved.description.as_str()), ("1.5", "Mine"));
        assert!(saved.settings.bleed.use_document, "the rest comes from Press Quality");
        // Edit… renames it; a new preset can't take a taken name.
        press(&mut app, ManagerAction::Edit);
        set_editor(&mut app, "name", json!("Press")).unwrap();
        assert!(confirm_editor(&mut app));
        assert_eq!(app.session.prefs.pdf_presets[0].name, "Press");
        press(&mut app, ManagerAction::New);
        set_editor(&mut app, "name", json!("press")).unwrap();
        assert!(!confirm_editor(&mut app));
        assert!(app.dialogs().get::<HostedPdfExport>().is_some(), "stays open on a taken name");
        // Built-in presets can't be edited; Delete selects the preset above.
        assert!(open_preset(&mut app, &json!({"name": "Press Quality"})).is_err());
        open(&mut app, Some("Press"));
        press(&mut app, ManagerAction::Delete);
        assert!(app.session.prefs.pdf_presets.is_empty());
        assert_eq!(manager_fields(&mut app)["selected"], "PDF/X-4:2010");
        let (id, ctx) = (dialog_id(KEY), egui::Context::default());
        app.dialogs().confirm_by_id(&ctx, id);
        assert!(app.dialogs().get_by_id::<PresetManager>(id).is_none(), "Close");
    }

    #[test]
    fn import_and_export_go_through_files() {
        let dir = std::env::temp_dir().join(format!("vc-pdfpresets-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mine.vcpdfpresets").to_string_lossy().to_string();
        let services = crate::Services {
            write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("pdf.preset.save", json!({"name": "Mine", "thumbnails": true})).unwrap();
        open(&mut app, Some("Mine"));
        // PLAN: assumed files-w7's `tests::picks::scripted(app) -> Picker` and `Picker::answer_saved(app, path)`.
        let picker = crate::tests::picks::scripted(&mut app);
        press(&mut app, ManagerAction::Export);
        picker.answer_saved(&mut app, &file);
        // Opening the file imports it (also how the web's picked file arrives).
        let bytes = std::fs::read(&file).unwrap();
        crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
        let names: Vec<&str> = app.session.prefs.pdf_presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Mine", "Mine 2"]);
        assert!(app.session.prefs.pdf_presets[1].settings.thumbnails);
        assert!(app.ui.status.contains("PDF presets"), "{}", app.ui.status);
        let _ = std::fs::remove_dir_all(dir);
    }
}
