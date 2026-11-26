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
use vectorcraft_engine::cmd::pdfcmds::{PRESET_EXTS, PRESET_FORMAT};
use vectorcraft_pdf::{APP, builtin_preset, builtin_presets, default_preset_name};

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
    preset_manager::open(app, &SOURCE, selected.unwrap_or(&default_preset_name(APP)));
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
    let default = default_preset_name(APP);
    let base = builtin_preset(&default, APP).map(|p| p.settings).unwrap_or_default();
    let mut out = vec![Detail::Note(crate::i18n::fmt(tl!("Differences from {preset}:"), &[("preset", default.as_str())]).to_string())];
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
            let above = preset_manager::above(i, builtin_presets(APP).len());
            Ok(app.session.pdf_presets().get(above).map(|p| p.name.clone()))
        }
        ManagerAction::Import => preset_manager::import(app, KEY, "pdf.preset.import", "PDF presets", PRESET_EXTS),
        ManagerAction::Export => preset_manager::export(app, "pdf.preset.export", PRESET_FORMAT, current),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::{DialogHost, StackDialog as _, dialog_id};
    use craft::ui::patterns::form_dialogs::HostedPdfExport;
    use vectorcraft_engine::Session;

    use super::super::preset_manager::PresetManager;

    fn app() -> VectorcraftApp {
        crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()))
    }

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().expect("the test app has a window")
    }

    /// Runs `f` on the window's dialog host.
    fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        app.dialogs().expect("the test app has a window").with(f)
    }

    /// One frame: the host's pump carries out what the manager queued.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = ctx(app);
        super::super::host::pump(app, &ctx);
    }

    /// Press a manager button as the user does: the manager queues it and the pump carries it out.
    fn press(app: &mut VectorcraftApp, action: ManagerAction) {
        host(app, |h| h.get_mut_by_id::<PresetManager>(dialog_id(KEY)).expect("manager is open").queue(ManagerEvent::Action(action)));
        frame(app);
    }

    fn manager_fields(app: &mut VectorcraftApp) -> Value {
        host(app, |h| h.get_by_id::<PresetManager>(dialog_id(KEY)).and_then(|m| m.json_fields())).expect("manager is open")
    }

    fn manager_open(app: &VectorcraftApp) -> bool {
        host(app, |h| h.get_by_id::<PresetManager>(dialog_id(KEY)).is_some())
    }

    fn editor_open(app: &VectorcraftApp) -> bool {
        host(app, |h| h.get::<HostedPdfExport>().is_some())
    }

    fn editor_fields(app: &mut VectorcraftApp) -> Value {
        host(app, |h| h.get::<HostedPdfExport>().and_then(|e| e.json_fields())).expect("editor is open")
    }

    fn set_editor(app: &mut VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let ctx = ctx(app);
        host(app, |h| {
            let id = h.get::<HostedPdfExport>().map(|e| e.id()).expect("editor is open");
            h.set_json_field(&ctx, id, field, value)
        })
    }

    /// OK in the editor, then the pump that saves it and reopens the manager.
    /// OK of the editor; true once it closed. Its OK is held: the pump answers it, and the press after the answer
    /// closes the dialog.
    fn confirm_editor(app: &mut VectorcraftApp) -> bool {
        let ctx = ctx(app);
        for _ in 0..2 {
            let closed = host(app, |h| h.confirm::<HostedPdfExport>(&ctx));
            super::super::host::pump(app, &ctx);
            if closed {
                return true;
            }
        }
        false
    }

    #[test]
    fn new_edit_and_delete_go_through_the_preset_editor() {
        let mut app = app();
        open(&mut app, Some("Press Quality"));
        assert_eq!(manager_fields(&mut app)["selected"], "Press Quality");
        // New… opens the editor on a copy of the selected preset; its OK saves it and comes back.
        press(&mut app, ManagerAction::New);
        assert!(!manager_open(&app), "the manager makes room for the editor");
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
        assert!(editor_open(&app), "stays open on a taken name");
        // Built-in presets can't be edited; Delete selects the preset above.
        assert!(open_preset(&mut app, &json!({"name": "Press Quality"})).is_err());
        open(&mut app, Some("Press"));
        press(&mut app, ManagerAction::Delete);
        assert!(app.session.prefs.pdf_presets.is_empty());
        assert_eq!(manager_fields(&mut app)["selected"], "PDF/X-4:2010");
        let ctx = ctx(&app);
        host(&app, |h| h.confirm_by_id(&ctx, dialog_id(KEY)));
        assert!(!manager_open(&app), "Close");
    }

    #[test]
    fn import_and_export_go_through_files() {
        let dir = std::env::temp_dir().join(format!("vc-pdfpresets-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mine.vcpdfpresets").to_string_lossy().to_string();
        let mut app = app();
        let picker = crate::tests_picks::scripted(&mut app);
        app.services.write = Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string())));
        app.run("pdf.preset.save", json!({"name": "Mine", "thumbnails": true})).unwrap();
        open(&mut app, Some("Mine"));
        press(&mut app, ManagerAction::Export);
        // The save panel suggests the preset's name.
        assert_eq!(picker.requests().last().and_then(|r| r.file_name.clone()).as_deref(), Some("Mine.vcpdfpresets"));
        picker.answer_saved(&file, None);
        let ctx = ctx(&app);
        crate::tests_picks::poll(&mut app, &ctx);
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
